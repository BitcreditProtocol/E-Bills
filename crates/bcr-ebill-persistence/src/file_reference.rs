use async_trait::async_trait;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::{
        Name, Sha256Hash,
        file_reference::{FileReference, FileReferenceContext},
    },
};
use bitcoin::hashes::sha256::Hash as Sha256HexHash;

use super::Result;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait FileReferenceStoreApi: ServiceTraitBounds {
    async fn upsert(
        &self,
        hash: &Sha256Hash,
        nostr_hash: &Sha256HexHash,
        name: Option<Name>,
        server_urls: Vec<url::Url>,
        is_important: Option<bool>,
        context: Vec<FileReferenceContext>,
    ) -> Result<FileReference>;

    async fn get(&self, hash: &Sha256Hash) -> Result<Option<FileReference>>;

    async fn find_by_nostr_hash(&self, nostr_hash: &Sha256HexHash)
    -> Result<Option<FileReference>>;

    async fn delete(&self, hash: &Sha256Hash) -> Result<()>;

    async fn list(&self) -> Result<Vec<FileReference>>;

    async fn list_important(&self) -> Result<Vec<FileReference>>;

    async fn add_server_urls(&self, hash: &Sha256Hash, urls: Vec<url::Url>) -> Result<bool>;

    async fn mark_important(&self, hash: &Sha256Hash, important: bool) -> Result<()>;

    async fn update_nostr_hash(&self, hash: &Sha256Hash, nostr_hash: &Sha256HexHash) -> Result<()>;

    async fn add_context(&self, hash: &Sha256Hash, context: FileReferenceContext) -> Result<bool>;

    async fn remove_context(
        &self,
        hash: &Sha256Hash,
        context: &FileReferenceContext,
    ) -> Result<bool>;
}
