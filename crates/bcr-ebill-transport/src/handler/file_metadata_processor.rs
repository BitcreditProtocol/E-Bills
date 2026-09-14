use crate::Result;
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::Sha256Hash};
use bcr_ebill_persistence::FileReferenceStoreApi;
use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use log::{debug, trace, warn};
use nostr::event::Event;
use std::sync::Arc;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait FileMetadataProcessorApi: ServiceTraitBounds {
    async fn process_file_metadata(&self, event: Box<Event>, node_id: &NodeId) -> Result<()>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockFileMetadataProcessorApi {}

pub struct FileMetadataProcessor {
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
}

impl FileMetadataProcessor {
    pub fn new(file_reference_store: Arc<dyn FileReferenceStoreApi>) -> Self {
        Self {
            file_reference_store,
        }
    }

    async fn find_file_by_nostr_hash(&self, nostr_hash: &Sha256HexHash) -> Option<Sha256Hash> {
        match self
            .file_reference_store
            .find_by_nostr_hash(nostr_hash)
            .await
        {
            Ok(Some(file)) => Some(file.hash),
            Ok(None) => None,
            Err(e) => {
                warn!("Error querying file reference by nostr_hash: {}", e);
                None
            }
        }
    }
}

impl ServiceTraitBounds for FileMetadataProcessor {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl FileMetadataProcessorApi for FileMetadataProcessor {
    async fn process_file_metadata(&self, event: Box<Event>, node_id: &NodeId) -> Result<()> {
        trace!(
            "Processing file metadata event {} from {} on node {}",
            event.id, event.pubkey, node_id
        );

        let mut ox_hash: Option<String> = None;
        let mut x_hash: Option<String> = None;
        let mut urls: Vec<url::Url> = Vec::new();

        for tag in event.tags.iter() {
            let tag_slice: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
            match tag_slice.as_slice() {
                ["ox", hash] => {
                    ox_hash = Some(hash.to_string());
                }
                ["x", hash] => {
                    x_hash = Some(hash.to_string());
                }
                ["url", url_str] => {
                    if let Ok(url) = url::Url::parse(url_str) {
                        if is_valid_file_url(&url) {
                            urls.push(url);
                        } else {
                            debug!("Ignoring non-HTTP(S) URL in file metadata: {}", url);
                        }
                    } else {
                        debug!("Malformed URL in file metadata event: {}", url_str);
                    }
                }
                ["fallback", url_str] => {
                    if let Ok(url) = url::Url::parse(url_str) {
                        if is_valid_file_url(&url) {
                            urls.push(url);
                        } else {
                            debug!("Ignoring non-HTTP(S) fallback URL: {}", url);
                        }
                    } else {
                        debug!("Malformed fallback URL in file metadata event: {}", url_str);
                    }
                }
                _ => {}
            }
        }

        if urls.is_empty() {
            debug!(
                "No valid URLs found in file metadata event {} from {}",
                event.id, event.pubkey
            );
            return Ok(());
        }

        let matched_hash = if let Some(ref ox) = ox_hash {
            match Sha256Hash::try_from(ox.clone()) {
                Ok(hash) => {
                    debug!("Trying to match file by ox hash: {}", hash);
                    match self.file_reference_store.get(&hash).await {
                        Ok(Some(_)) => {
                            debug!("Found existing file reference for ox hash: {}", hash);
                            Some(hash)
                        }
                        Ok(None) => {
                            debug!("No file reference found for ox hash: {}", hash);
                            None
                        }
                        Err(e) => {
                            warn!("Error looking up file by ox hash: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    debug!("Invalid ox hash format '{}': {}", ox, e);
                    None
                }
            }
        } else {
            None
        };

        let matched_hash = if matched_hash.is_none() {
            if let Some(ref x) = x_hash {
                match x.parse::<Sha256HexHash>() {
                    Ok(nostr_hash) => {
                        debug!("Trying to match file by x (nostr_hash): {}", nostr_hash);
                        match self.find_file_by_nostr_hash(&nostr_hash).await {
                            Some(hash) => {
                                debug!("Found existing file reference for x hash: {}", hash);
                                Some(hash)
                            }
                            None => {
                                debug!(
                                    "No file reference found for x (nostr_hash): {}",
                                    nostr_hash
                                );
                                None
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Invalid x hash format '{}': {}", x, e);
                        None
                    }
                }
            } else {
                None
            }
        } else {
            matched_hash
        };

        match matched_hash {
            Some(file_hash) => {
                match self
                    .file_reference_store
                    .add_server_urls(&file_hash, urls.clone())
                    .await
                {
                    Ok(added) => {
                        if added {
                            debug!(
                                "Merged {} new server URLs into file reference {} from event {}",
                                urls.len(),
                                file_hash,
                                event.id
                            );
                        } else {
                            debug!(
                                "No new URLs to add for file reference {} from event {} (all duplicates)",
                                file_hash, event.id
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to merge server URLs for file {} from event {}: {}",
                            file_hash, event.id, e
                        );
                    }
                }
            }
            None => {
                debug!(
                    "Skipping file metadata event {} - no matching local file reference found (ox={:?}, x={:?})",
                    event.id, ox_hash, x_hash
                );
            }
        }

        Ok(())
    }
}

fn is_valid_file_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core::NodeId;
    use bcr_ebill_core::protocol::{
        Name,
        file_reference::{FileReference, FileReferenceContext},
    };
    use nostr::event::FinalizeEvent;
    use std::str::FromStr;

    mockall::mock! {
        pub FileReferenceStore {}

        #[async_trait::async_trait]
        impl FileReferenceStoreApi for FileReferenceStore {
            async fn upsert(&self, hash: &Sha256Hash, nostr_hash: &Sha256HexHash, name: Option<Name>, server_urls: Vec<url::Url>, is_important: Option<bool>, context: Vec<FileReferenceContext>) -> std::result::Result<FileReference, bcr_ebill_persistence::Error>;
            async fn get(&self, hash: &Sha256Hash) -> std::result::Result<Option<FileReference>, bcr_ebill_persistence::Error>;
            async fn find_by_nostr_hash(&self, nostr_hash: &Sha256HexHash) -> std::result::Result<Option<FileReference>, bcr_ebill_persistence::Error>;
            async fn delete(&self, hash: &Sha256Hash) -> std::result::Result<(), bcr_ebill_persistence::Error>;
            async fn list(&self) -> std::result::Result<Vec<FileReference>, bcr_ebill_persistence::Error>;
            async fn list_important(&self) -> std::result::Result<Vec<FileReference>, bcr_ebill_persistence::Error>;
            async fn add_server_urls(&self, hash: &Sha256Hash, urls: Vec<url::Url>) -> std::result::Result<bool, bcr_ebill_persistence::Error>;
            async fn mark_important(&self, hash: &Sha256Hash, important: bool) -> std::result::Result<(), bcr_ebill_persistence::Error>;
            async fn update_nostr_hash(&self, hash: &Sha256Hash, nostr_hash: &Sha256HexHash) -> std::result::Result<(), bcr_ebill_persistence::Error>;
            async fn add_context(&self, hash: &Sha256Hash, context: FileReferenceContext) -> std::result::Result<bool, bcr_ebill_persistence::Error>;
            async fn remove_context(&self, hash: &Sha256Hash, context: &FileReferenceContext) -> std::result::Result<bool, bcr_ebill_persistence::Error>;
        }
    }

    impl ServiceTraitBounds for MockFileReferenceStore {}

    fn test_hash() -> Sha256Hash {
        Sha256Hash::new("test_hash_12345678901234567890123456789012")
    }

    fn test_nostr_hash() -> Sha256HexHash {
        Sha256HexHash::from_str("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap()
    }

    async fn create_test_event_with_tags(tags: Vec<Vec<&str>>) -> Box<Event> {
        use nostr::{event::Kind, key::Keys};

        let keys = Keys::generate();
        let mut event_builder = nostr::event::EventBuilder::new(Kind::Custom(1063), "");

        for tag in tags {
            event_builder = event_builder.tags([nostr::event::Tag::parse(tag).unwrap()]);
        }

        let event = event_builder.finalize(&keys).expect("to sign event");
        Box::new(event)
    }

    fn node_id_test() -> NodeId {
        use bcr_ebill_core::protocol::crypto::BcrKeys;
        NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet)
    }

    fn create_test_file_reference(hash: Sha256Hash, nostr_hash: Sha256HexHash) -> FileReference {
        FileReference::new(hash, nostr_hash, Some(Name::new("test.txt").unwrap()))
    }

    #[tokio::test]
    async fn kind_1063_ingest_with_ox_match() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .with(mockall::predicate::eq(hash.clone()))
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .with(
                mockall::predicate::eq(hash.clone()),
                mockall::predicate::function(|urls: &Vec<url::Url>| {
                    urls.len() == 1 && urls[0].as_str() == "https://example.com/file.txt"
                }),
            )
            .returning(|_, _| Ok(true));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn kind_1063_ingest_unknown_file_skipped() {
        let hash = test_hash();

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .with(mockall::predicate::eq(hash.clone()))
            .returning(|_| Ok(None));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn kind_1063_ingest_with_fallback_urls() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .with(
                mockall::predicate::eq(hash.clone()),
                mockall::predicate::function(|urls: &Vec<url::Url>| urls.len() == 2),
            )
            .returning(|_, _| Ok(true));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
            vec!["fallback", "https://backup.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn kind_1063_ingest_ignores_non_http_urls() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .with(
                mockall::predicate::eq(hash.clone()),
                mockall::predicate::function(|urls: &Vec<url::Url>| {
                    urls.len() == 1 && urls[0].as_str() == "https://example.com/file.txt"
                }),
            )
            .returning(|_, _| Ok(true));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "ftp://example.com/file.txt"],
            vec!["url", "https://example.com/file.txt"],
            vec!["url", "file:///local/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn kind_1063_ingest_x_tag_fallback() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_find_by_nostr_hash()
            .with(mockall::predicate::eq(nostr_hash))
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .with(
                mockall::predicate::eq(hash.clone()),
                mockall::predicate::function(|urls: &Vec<url::Url>| {
                    urls.len() == 1 && urls[0].as_str() == "https://example.com/file.txt"
                }),
            )
            .returning(|_, _| Ok(true));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["x", &nostr_hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn kind_1063_ingest_url_deduplication() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(false));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn malformed_kind_1063_is_ignored_without_side_effects() {
        let hash = test_hash();

        let mut mock_store = MockFileReferenceStore::new();
        mock_store.expect_get().returning(|_| Ok(None));
        mock_store.expect_add_server_urls().never();
        mock_store.expect_upsert().never();

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "not-a-valid-url"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn malformed_ox_hash_is_ignored_safely() {
        let mut mock_store = MockFileReferenceStore::new();
        mock_store.expect_get().never();
        mock_store.expect_list().never();
        mock_store.expect_add_server_urls().never();

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", "not-a-valid-hash"],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn duplicate_url_announcements_remain_deduplicated() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let file = create_test_file_reference(hash.clone(), nostr_hash);

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .returning(move |_| Ok(Some(file.clone())));
        mock_store
            .expect_add_server_urls()
            .with(
                mockall::predicate::eq(hash.clone()),
                mockall::predicate::function(|urls: &Vec<url::Url>| {
                    urls.len() == 3
                        && urls
                            .iter()
                            .all(|u| u.as_str() == "https://example.com/file.txt")
                }),
            )
            .returning(|_, _| Ok(true));

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
            vec!["url", "https://example.com/file.txt"],
            vec!["fallback", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn trust_scope_unknown_file_no_new_reference_created() {
        let hash = test_hash();

        let mut mock_store = MockFileReferenceStore::new();
        mock_store
            .expect_get()
            .with(mockall::predicate::eq(hash.clone()))
            .returning(|_| Ok(None));
        mock_store.expect_upsert().never();
        mock_store.expect_add_server_urls().never();

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["ox", &hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }

    #[tokio::test]
    async fn malicious_x_tag_no_file_created_only_known_files_updated() {
        let unknown_nostr_hash = Sha256HexHash::from_str(
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210",
        )
        .unwrap();

        let mut mock_store = MockFileReferenceStore::new();
        mock_store.expect_get().returning(|_| Ok(None));
        mock_store
            .expect_find_by_nostr_hash()
            .with(mockall::predicate::eq(unknown_nostr_hash))
            .returning(|_| Ok(None));
        mock_store.expect_upsert().never();
        mock_store.expect_add_server_urls().never();

        let processor = FileMetadataProcessor::new(Arc::new(mock_store));

        let event = create_test_event_with_tags(vec![
            vec!["x", &unknown_nostr_hash.to_string()],
            vec!["url", "https://example.com/file.txt"],
        ])
        .await;

        processor
            .process_file_metadata(event, &node_id_test())
            .await
            .expect("processing failed");
    }
}
