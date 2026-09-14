use crate::{
    Error, Result,
    db::surreal::{Bindings, SurrealWrapper},
    file_reference::FileReferenceStoreApi,
};
use async_trait::async_trait;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::{
        Name, Sha256Hash, Timestamp,
        file_reference::{FileReference, FileReferenceContext},
    },
};
use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

#[derive(Clone)]
pub struct SurrealFileReferenceStore {
    db: SurrealWrapper,
}

impl SurrealFileReferenceStore {
    const TABLE: &'static str = "file_reference";

    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }

    fn thing_id(hash: &Sha256Hash) -> Thing {
        Thing::from((Self::TABLE.to_owned(), hash.to_string()))
    }
}

impl ServiceTraitBounds for SurrealFileReferenceStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl FileReferenceStoreApi for SurrealFileReferenceStore {
    async fn upsert(
        &self,
        hash: &Sha256Hash,
        nostr_hash: &Sha256HexHash,
        name: Option<Name>,
        server_urls: Vec<url::Url>,
        is_important: Option<bool>,
        context: Vec<FileReferenceContext>,
    ) -> Result<FileReference> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        let now = Timestamp::now();
        let db_record = match existing {
            Some(mut existing) => {
                existing.nostr_hash = nostr_hash.to_string();
                if let Some(n) = name {
                    existing.name = Some(n);
                }
                for url in server_urls {
                    add_url_deduped(&mut existing.server_urls, url);
                }
                if let Some(important) = is_important {
                    existing.is_important = important;
                }
                for ctx in context {
                    if !existing.context.contains(&ctx) {
                        existing.context.push(ctx);
                    }
                }
                existing.updated_at = now;
                existing
            }
            None => {
                let mut deduped_urls = Vec::new();
                for url in server_urls {
                    add_url_deduped(&mut deduped_urls, url);
                }
                FileReferenceDb {
                    id: Self::thing_id(hash),
                    hash: hash.clone(),
                    nostr_hash: nostr_hash.to_string(),
                    name,
                    server_urls: deduped_urls,
                    is_important: is_important.unwrap_or(false),
                    context,
                    created_at: now,
                    updated_at: now,
                }
            }
        };

        let _: Option<FileReferenceDb> = self
            .db
            .upsert(Self::TABLE, hash.to_string(), db_record.clone())
            .await?;

        Ok(db_record.try_into()?)
    }

    async fn get(&self, hash: &Sha256Hash) -> Result<Option<FileReference>> {
        let result: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;
        match result {
            Some(db) => Ok(Some(db.try_into()?)),
            None => Ok(None),
        }
    }

    async fn find_by_nostr_hash(
        &self,
        nostr_hash: &Sha256HexHash,
    ) -> Result<Option<FileReference>> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::TABLE)?;
        bindings.add("nostr_hash", nostr_hash.to_string())?;
        let query =
            "SELECT * FROM type::table($table) WHERE nostr_hash = $nostr_hash LIMIT 1".to_string();
        let mut results: Vec<FileReferenceDb> = self.db.query(&query, bindings).await?;

        match results.pop() {
            Some(db) => Ok(Some(db.try_into()?)),
            None => Ok(None),
        }
    }

    async fn delete(&self, hash: &Sha256Hash) -> Result<()> {
        let _: Option<FileReferenceDb> = self.db.delete(Self::TABLE, hash.to_string()).await?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<FileReference>> {
        let results: Vec<FileReferenceDb> = self.db.select_all(Self::TABLE).await?;
        results
            .into_iter()
            .map(|r| r.try_into())
            .collect::<Result<Vec<_>>>()
    }

    async fn list_important(&self) -> Result<Vec<FileReference>> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::TABLE)?;
        let query = "SELECT * FROM type::table($table) WHERE is_important = true".to_string();
        let results: Vec<FileReferenceDb> = self.db.query(&query, bindings).await?;
        results
            .into_iter()
            .map(|r| r.try_into())
            .collect::<Result<Vec<_>>>()
    }

    async fn add_server_urls(&self, hash: &Sha256Hash, urls: Vec<url::Url>) -> Result<bool> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        match existing {
            Some(mut db_record) => {
                let original_len = db_record.server_urls.len();
                for url in urls {
                    add_url_deduped(&mut db_record.server_urls, url);
                }
                let added = db_record.server_urls.len() > original_len;
                if added {
                    db_record.updated_at = Timestamp::now();
                    let _: Option<FileReferenceDb> = self
                        .db
                        .upsert(Self::TABLE, hash.to_string(), db_record)
                        .await?;
                }
                Ok(added)
            }
            None => Ok(false),
        }
    }

    async fn mark_important(&self, hash: &Sha256Hash, important: bool) -> Result<()> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        if let Some(mut db_record) = existing
            && db_record.is_important != important
        {
            db_record.is_important = important;
            db_record.updated_at = Timestamp::now();
            let _: Option<FileReferenceDb> = self
                .db
                .upsert(Self::TABLE, hash.to_string(), db_record)
                .await?;
        }
        Ok(())
    }

    async fn update_nostr_hash(&self, hash: &Sha256Hash, nostr_hash: &Sha256HexHash) -> Result<()> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        if let Some(mut db_record) = existing
            && db_record.nostr_hash != nostr_hash.to_string()
        {
            db_record.nostr_hash = nostr_hash.to_string();
            db_record.updated_at = Timestamp::now();
            let _: Option<FileReferenceDb> = self
                .db
                .upsert(Self::TABLE, hash.to_string(), db_record)
                .await?;
        }
        Ok(())
    }

    async fn add_context(&self, hash: &Sha256Hash, context: FileReferenceContext) -> Result<bool> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        match existing {
            Some(mut db_record) => {
                if !db_record.context.contains(&context) {
                    db_record.context.push(context);
                    db_record.updated_at = Timestamp::now();
                    let _: Option<FileReferenceDb> = self
                        .db
                        .upsert(Self::TABLE, hash.to_string(), db_record)
                        .await?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            None => Ok(false),
        }
    }

    async fn remove_context(
        &self,
        hash: &Sha256Hash,
        context: &FileReferenceContext,
    ) -> Result<bool> {
        let existing: Option<FileReferenceDb> =
            self.db.select_one(Self::TABLE, hash.to_string()).await?;

        match existing {
            Some(mut db_record) => {
                if let Some(pos) = db_record.context.iter().position(|c| c == context) {
                    db_record.context.remove(pos);
                    db_record.updated_at = Timestamp::now();
                    let _: Option<FileReferenceDb> = self
                        .db
                        .upsert(Self::TABLE, hash.to_string(), db_record)
                        .await?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            None => Ok(false),
        }
    }
}

fn add_url_deduped(urls: &mut Vec<url::Url>, url: url::Url) {
    let normalized = normalize_url(&url);
    if !urls.iter().any(|u| urls_equal(u, &normalized)) {
        urls.push(url);
    }
}

fn urls_equal(a: &url::Url, b: &str) -> bool {
    normalize_url(a) == b
}

fn normalize_url(url: &url::Url) -> String {
    let mut s = url.to_string();
    if s.ends_with('/') {
        s.pop();
    }
    s.to_lowercase()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReferenceDb {
    pub id: Thing,
    pub hash: Sha256Hash,
    pub nostr_hash: String,
    pub name: Option<Name>,
    pub server_urls: Vec<url::Url>,
    pub is_important: bool,
    pub context: Vec<FileReferenceContext>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl TryFrom<FileReferenceDb> for FileReference {
    type Error = Error;

    fn try_from(db: FileReferenceDb) -> Result<Self> {
        let nostr_hash = db
            .nostr_hash
            .parse::<Sha256HexHash>()
            .map_err(|_| Error::EncodingError)?;

        Ok(Self {
            hash: db.hash,
            nostr_hash,
            name: db.name,
            server_urls: db.server_urls,
            is_important: db.is_important,
            context: db.context,
            created_at: db.created_at,
            updated_at: db.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::get_memory_db;

    fn test_hash() -> Sha256Hash {
        Sha256Hash::new("test_hash_12345678901234567890123456789012")
    }

    fn test_nostr_hash() -> Sha256HexHash {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .unwrap()
    }

    fn test_nostr_hash_2() -> Sha256HexHash {
        "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
            .parse()
            .unwrap()
    }

    async fn get_store() -> SurrealFileReferenceStore {
        let mem_db = get_memory_db("test", "file_reference")
            .await
            .expect("could not create memory db");
        SurrealFileReferenceStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    #[tokio::test]
    async fn test_upsert_creates_new() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let name = Name::new("test_file.txt").unwrap();
        let url = url::Url::parse("https://blossom.example.com").unwrap();

        let result = store
            .upsert(
                &hash,
                &nostr_hash,
                Some(name.clone()),
                vec![url],
                Some(true),
                vec![],
            )
            .await
            .expect("upsert failed");

        assert_eq!(result.hash, hash);
        assert_eq!(result.nostr_hash, nostr_hash);
        assert_eq!(result.name, Some(name));
        assert_eq!(result.server_urls.len(), 1);
        assert!(result.is_important);
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let nostr_hash_2 = test_nostr_hash_2();
        let name = Name::new("test_file.txt").unwrap();
        let name_2 = Name::new("updated_file.txt").unwrap();
        let url1 = url::Url::parse("https://blossom1.example.com").unwrap();
        let url2 = url::Url::parse("https://blossom2.example.com").unwrap();

        store
            .upsert(
                &hash,
                &nostr_hash,
                Some(name.clone()),
                vec![url1.clone()],
                Some(false),
                vec![],
            )
            .await
            .expect("first upsert failed");

        let result = store
            .upsert(
                &hash,
                &nostr_hash_2,
                Some(name_2.clone()),
                vec![url2.clone()],
                Some(true),
                vec![],
            )
            .await
            .expect("second upsert failed");

        assert_eq!(result.nostr_hash, nostr_hash_2);
        assert_eq!(result.name, Some(name_2));
        assert_eq!(result.server_urls.len(), 2);
        assert!(result.server_urls.contains(&url1));
        assert!(result.server_urls.contains(&url2));
        assert!(result.is_important);
    }

    #[tokio::test]
    async fn test_find_by_nostr_hash() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        store
            .upsert(&hash, &nostr_hash, None, vec![], Some(false), vec![])
            .await
            .expect("upsert failed");

        let result = store
            .find_by_nostr_hash(&nostr_hash)
            .await
            .expect("find_by_nostr_hash failed");

        assert!(result.is_some());
        assert_eq!(result.unwrap().hash, hash);
    }

    #[tokio::test]
    async fn test_find_by_nostr_hash_missing() {
        let store = get_store().await;

        let result = store
            .find_by_nostr_hash(&test_nostr_hash())
            .await
            .expect("find_by_nostr_hash failed");

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_upsert_deduplicates_server_urls() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        let url1 = url::Url::parse("https://blossom.example.com").unwrap();
        let url2 = url::Url::parse("https://blossom.example.com/").unwrap();
        let url3 = url::Url::parse("https://blossom2.example.com").unwrap();

        let result = store
            .upsert(
                &hash,
                &nostr_hash,
                None,
                vec![url1, url2, url3],
                None,
                vec![],
            )
            .await
            .expect("upsert failed");

        assert_eq!(result.server_urls.len(), 2);
    }

    #[tokio::test]
    async fn test_get_existing() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let name = Name::new("test_file.txt").unwrap();

        store
            .upsert(
                &hash,
                &nostr_hash,
                Some(name.clone()),
                vec![],
                Some(false),
                vec![],
            )
            .await
            .expect("upsert failed");

        let result = store.get(&hash).await.expect("get failed");

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.hash, hash);
        assert_eq!(result.name, Some(name));
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let store = get_store().await;
        let hash = test_hash();

        let result = store.get(&hash).await.expect("get failed");

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        store
            .upsert(&hash, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("upsert failed");

        assert!(store.get(&hash).await.expect("get failed").is_some());

        store.delete(&hash).await.expect("delete failed");

        assert!(store.get(&hash).await.expect("get failed").is_none());
    }

    #[tokio::test]
    async fn test_list() {
        let store = get_store().await;
        let hash1 = Sha256Hash::new("test_hash_12345678901234567890123456789011");
        let hash2 = Sha256Hash::new("test_hash_12345678901234567890123456789022");
        let nostr_hash = test_nostr_hash();

        store
            .upsert(&hash1, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("upsert 1 failed");
        store
            .upsert(&hash2, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("upsert 2 failed");

        let results = store.list().await.expect("list failed");

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_list_important() {
        let store = get_store().await;
        let hash1 = Sha256Hash::new("test_hash_12345678901234567890123456789011");
        let hash2 = Sha256Hash::new("test_hash_12345678901234567890123456789022");
        let nostr_hash = test_nostr_hash();

        store
            .upsert(&hash1, &nostr_hash, None, vec![], Some(true), vec![])
            .await
            .expect("upsert 1 failed");
        store
            .upsert(&hash2, &nostr_hash, None, vec![], Some(false), vec![])
            .await
            .expect("upsert 2 failed");

        let results = store.list_important().await.expect("list_important failed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hash, hash1);
    }

    #[tokio::test]
    async fn test_add_server_urls() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let url1 = url::Url::parse("https://blossom1.example.com").unwrap();

        store
            .upsert(&hash, &nostr_hash, None, vec![url1], None, vec![])
            .await
            .expect("upsert failed");

        let url2 = url::Url::parse("https://blossom2.example.com").unwrap();
        let added = store
            .add_server_urls(&hash, vec![url2])
            .await
            .expect("add_server_urls failed");

        assert!(added);

        let result = store.get(&hash).await.expect("get failed").unwrap();
        assert_eq!(result.server_urls.len(), 2);
    }

    #[tokio::test]
    async fn test_add_server_urls_no_change_for_duplicates() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let url1 = url::Url::parse("https://blossom.example.com").unwrap();

        store
            .upsert(&hash, &nostr_hash, None, vec![url1.clone()], None, vec![])
            .await
            .expect("upsert failed");

        let added = store
            .add_server_urls(&hash, vec![url1])
            .await
            .expect("add_server_urls failed");

        assert!(!added);
    }

    #[tokio::test]
    async fn test_mark_important() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        store
            .upsert(&hash, &nostr_hash, None, vec![], Some(false), vec![])
            .await
            .expect("upsert failed");

        store
            .mark_important(&hash, true)
            .await
            .expect("mark_important failed");

        let result = store.get(&hash).await.expect("get failed").unwrap();
        assert!(result.is_important);
    }

    #[tokio::test]
    async fn test_update_nostr_hash() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let nostr_hash_2 = test_nostr_hash_2();

        store
            .upsert(&hash, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("upsert failed");

        store
            .update_nostr_hash(&hash, &nostr_hash_2)
            .await
            .expect("update_nostr_hash failed");

        let result = store.get(&hash).await.expect("get failed").unwrap();
        assert_eq!(result.nostr_hash, nostr_hash_2);
    }

    #[tokio::test]
    async fn test_upsert_preserves_existing_name_when_none_provided() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let name = Name::new("original_name.txt").unwrap();

        store
            .upsert(&hash, &nostr_hash, Some(name.clone()), vec![], None, vec![])
            .await
            .expect("first upsert failed");

        let result = store
            .upsert(&hash, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("second upsert failed");

        assert_eq!(result.name, Some(name));
    }

    #[tokio::test]
    async fn test_coexistence_with_existing_file_fields() {
        let store = get_store().await;

        let file_hash = Sha256Hash::new("file_hash_12345678901234567890123456789012");
        let nostr_hash = test_nostr_hash();
        let name = Name::new("avatar.png").unwrap();
        let server_url = url::Url::parse("https://blossom.example.com").unwrap();

        let file_ref = store
            .upsert(
                &file_hash,
                &nostr_hash,
                Some(name.clone()),
                vec![server_url],
                Some(true),
                vec![],
            )
            .await
            .expect("upsert failed");

        assert_eq!(file_ref.hash, file_hash);
        assert_eq!(file_ref.nostr_hash, nostr_hash);
        assert_eq!(file_ref.name, Some(name));
        assert!(file_ref.is_important);

        let retrieved = store.get(&file_hash).await.expect("get failed");
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_upsert_with_context() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        let context = vec![
            FileReferenceContext::Identity {
                field: "avatar_file".to_string(),
            },
            FileReferenceContext::Company {
                company_id: "company123".to_string(),
                field: "logo_file".to_string(),
            },
        ];

        let result = store
            .upsert(&hash, &nostr_hash, None, vec![], None, context)
            .await
            .expect("upsert failed");

        assert_eq!(result.context.len(), 2);
    }

    #[tokio::test]
    async fn test_add_and_remove_context() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        // First upsert to create the record
        store
            .upsert(&hash, &nostr_hash, None, vec![], None, vec![])
            .await
            .expect("upsert failed");

        let context = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };

        // Add context
        let added = store
            .add_context(&hash, context.clone())
            .await
            .expect("add_context failed");
        assert!(added, "Context should have been added");

        let result = store.get(&hash).await.expect("get failed").unwrap();
        assert_eq!(result.context.len(), 1);
        assert!(result.context.contains(&context));

        // Remove context
        let removed = store
            .remove_context(&hash, &context)
            .await
            .expect("remove_context failed");
        assert!(removed, "Context should have been removed");

        let result = store.get(&hash).await.expect("get failed").unwrap();
        assert!(result.context.is_empty());
    }

    #[tokio::test]
    async fn test_context_deduplication_on_upsert() {
        let store = get_store().await;
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();

        let context1 = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };
        let context2 = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };

        store
            .upsert(
                &hash,
                &nostr_hash,
                None,
                vec![],
                None,
                vec![context1.clone()],
            )
            .await
            .expect("first upsert failed");

        let result = store
            .upsert(&hash, &nostr_hash, None, vec![], None, vec![context2])
            .await
            .expect("second upsert failed");

        assert_eq!(result.context.len(), 1);
    }

    #[tokio::test]
    async fn test_add_context_to_nonexistent_record() {
        let store = get_store().await;
        let hash = test_hash();

        let context = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };

        // Adding context to non-existent record should return false
        let added = store
            .add_context(&hash, context)
            .await
            .expect("add_context should not fail");
        assert!(!added, "Should not add context to non-existent record");
    }
}
