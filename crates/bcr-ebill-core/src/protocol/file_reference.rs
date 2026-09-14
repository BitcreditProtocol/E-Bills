use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use serde::{Deserialize, Serialize};

use crate::protocol::{Name, Sha256Hash, Timestamp};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileReference {
    pub hash: Sha256Hash,
    pub nostr_hash: Sha256HexHash,
    pub name: Option<Name>,
    pub server_urls: Vec<url::Url>,
    pub is_important: bool,
    pub context: Vec<FileReferenceContext>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl FileReference {
    pub fn new(hash: Sha256Hash, nostr_hash: Sha256HexHash, name: Option<Name>) -> Self {
        let now = Timestamp::now();
        Self {
            hash,
            nostr_hash,
            name,
            server_urls: Vec::new(),
            is_important: false,
            context: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn add_server_url(&mut self, url: url::Url) -> bool {
        if !self.server_urls.iter().any(|u| urls_equal(u, &url)) {
            self.server_urls.push(url);
            self.updated_at = Timestamp::now();
            true
        } else {
            false
        }
    }

    pub fn add_server_urls(&mut self, urls: Vec<url::Url>) {
        for url in urls {
            self.add_server_url(url);
        }
    }

    pub fn mark_important(&mut self, important: bool) {
        if self.is_important != important {
            self.is_important = important;
            self.updated_at = Timestamp::now();
        }
    }

    pub fn update_nostr_hash(&mut self, nostr_hash: Sha256HexHash) {
        if self.nostr_hash != nostr_hash {
            self.nostr_hash = nostr_hash;
            self.updated_at = Timestamp::now();
        }
    }

    pub fn update_name(&mut self, name: Option<Name>) {
        if name.is_some() && self.name != name {
            self.name = name;
            self.updated_at = Timestamp::now();
        }
    }

    pub fn add_context(&mut self, context: FileReferenceContext) {
        if !self.context.contains(&context) {
            self.context.push(context);
            self.updated_at = Timestamp::now();
        }
    }

    pub fn remove_context(&mut self, context: &FileReferenceContext) {
        if let Some(pos) = self.context.iter().position(|c| c == context) {
            self.context.remove(pos);
            self.updated_at = Timestamp::now();
        }
    }
}

fn urls_equal(a: &url::Url, b: &url::Url) -> bool {
    let a_norm = normalize_url(a);
    let b_norm = normalize_url(b);
    a_norm == b_norm
}

fn normalize_url(url: &url::Url) -> String {
    let mut s = url.to_string();
    if s.ends_with('/') {
        s.pop();
    }
    s.to_lowercase()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(tag = "type")]
pub enum FileReferenceContext {
    Identity { field: String },
    Company { company_id: String, field: String },
    Contact { node_id: String, field: String },
    Bill { bill_id: String, field: String },
    DirectUpload,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct FileReferenceUpsertInput {
    pub hash: Sha256Hash,
    pub nostr_hash: Sha256HexHash,
    pub name: Option<Name>,
    pub server_urls: Vec<url::Url>,
    pub is_important: Option<bool>,
    pub context: Vec<FileReferenceContext>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hash() -> Sha256Hash {
        Sha256Hash::new("test_hash_12345678901234567890123456789012")
    }

    fn test_nostr_hash() -> Sha256HexHash {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .unwrap()
    }

    #[test]
    fn test_file_reference_new() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let name = Some(Name::new("test_file.txt").unwrap());

        let ref_file = FileReference::new(hash.clone(), nostr_hash, name.clone());

        assert_eq!(ref_file.hash, hash);
        assert_eq!(ref_file.nostr_hash, nostr_hash);
        assert_eq!(ref_file.name, name);
        assert!(ref_file.server_urls.is_empty());
        assert!(!ref_file.is_important);
        assert!(ref_file.context.is_empty());
    }

    #[test]
    fn test_add_server_url_deduplicates() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        let url1 = url::Url::parse("https://blossom.example.com").unwrap();
        let url2 = url::Url::parse("https://blossom.example.com/").unwrap();

        assert!(ref_file.add_server_url(url1));
        assert!(!ref_file.add_server_url(url2));

        assert_eq!(ref_file.server_urls.len(), 1);
    }

    #[test]
    fn test_add_multiple_server_urls() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        let url1 = url::Url::parse("https://blossom1.example.com").unwrap();
        let url2 = url::Url::parse("https://blossom2.example.com").unwrap();
        let url3 = url::Url::parse("https://blossom1.example.com").unwrap();

        ref_file.add_server_urls(vec![url1, url2, url3]);

        assert_eq!(ref_file.server_urls.len(), 2);
    }

    #[test]
    fn test_mark_important() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        assert!(!ref_file.is_important);

        ref_file.mark_important(true);
        assert!(ref_file.is_important);

        let updated_at = ref_file.updated_at;
        ref_file.mark_important(true);
        assert_eq!(ref_file.updated_at, updated_at);
    }

    #[test]
    fn test_update_nostr_hash() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        let new_nostr_hash: Sha256HexHash =
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                .parse()
                .unwrap();

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        let old_updated_at = ref_file.updated_at;
        ref_file.update_nostr_hash(new_nostr_hash);

        assert_eq!(ref_file.nostr_hash, new_nostr_hash);
        assert!(ref_file.updated_at >= old_updated_at);
    }

    #[test]
    fn test_add_context() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        let context = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };

        ref_file.add_context(context.clone());
        assert_eq!(ref_file.context.len(), 1);
        assert!(ref_file.context.contains(&context));

        let updated_at = ref_file.updated_at;
        ref_file.add_context(context);
        assert_eq!(ref_file.context.len(), 1);
        assert_eq!(ref_file.updated_at, updated_at);
    }

    #[test]
    fn test_remove_context() {
        let hash = test_hash();
        let nostr_hash = test_nostr_hash();
        let mut ref_file = FileReference::new(hash, nostr_hash, None);

        let context = FileReferenceContext::Identity {
            field: "avatar_file".to_string(),
        };

        ref_file.add_context(context.clone());
        assert_eq!(ref_file.context.len(), 1);

        ref_file.remove_context(&context);
        assert!(ref_file.context.is_empty());
    }

    #[test]
    fn test_context_variants() {
        let identity_ctx = FileReferenceContext::Identity {
            field: "profile_picture_file".to_string(),
        };
        let company_ctx = FileReferenceContext::Company {
            company_id: "company123".to_string(),
            field: "logo_file".to_string(),
        };
        let contact_ctx = FileReferenceContext::Contact {
            node_id: "node456".to_string(),
            field: "avatar_file".to_string(),
        };
        let bill_ctx = FileReferenceContext::Bill {
            bill_id: "bill789".to_string(),
            field: "attachment".to_string(),
        };
        let upload_ctx = FileReferenceContext::DirectUpload;
        let unknown_ctx = FileReferenceContext::Unknown;

        assert_ne!(identity_ctx, company_ctx);
        assert_ne!(contact_ctx, bill_ctx);
        assert_ne!(upload_ctx, unknown_ctx);
    }
}
