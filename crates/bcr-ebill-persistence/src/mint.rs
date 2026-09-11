use super::Result;
use async_trait::async_trait;

use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::Sum,
    protocol::Timestamp,
    protocol::mint::{MintOffer, MintRequest, MintRequestStatus},
};
use uuid::Uuid;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait MintStoreApi: ServiceTraitBounds {
    /// Checks if there is any minting request for the given bill
    async fn exists_for_bill(&self, requester_node_id: &NodeId, bill_id: &BillId) -> Result<bool>;
    /// Checks if there is an active request to mint for the given bill and mint
    async fn get_requests(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
        mint_node_id: &NodeId,
    ) -> Result<Vec<MintRequest>>;
    /// Returns all mint requests, which are not finished (i.e. offered, accepted or pending)
    async fn get_all_active_requests(&self) -> Result<Vec<MintRequest>>;
    /// Checks if there is an active request to mint for the given bill
    async fn get_requests_for_bill(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
    ) -> Result<Vec<MintRequest>>;
    /// Adds a new request to mint for a bill and mint
    async fn add_request(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
        mint_node_id: &NodeId,
        mint_request_id: &Uuid,
        timestamp: Timestamp,
    ) -> Result<()>;
    /// Get request to mint for the given mint request id
    async fn get_request(&self, mint_request_id: &Uuid) -> Result<Option<MintRequest>>;
    /// Update the given request to mint with a new status
    async fn update_request(
        &self,
        mint_request_id: &Uuid,
        new_status: &MintRequestStatus,
    ) -> Result<()>;
    /// Adds proofs for a given offer
    async fn add_proofs_to_offer(&self, mint_request_id: &Uuid, proofs: &str) -> Result<()>;
    /// Adds recovery data to offer
    async fn add_recovery_data_to_offer(
        &self,
        mint_request_id: &Uuid,
        secrets: &[String],
        rs: &[String],
    ) -> Result<()>;
    /// Set proofs to spent for offer
    async fn set_proofs_to_spent_for_offer(&self, mint_request_id: &Uuid) -> Result<()>;
    /// Adds an offer for a request to mint
    async fn add_offer(
        &self,
        mint_request_id: &Uuid,
        keyset_id: &str,
        expiration_timestamp: Timestamp,
        discounted_sum: Sum,
    ) -> Result<()>;
    /// Gets an offer by the mint request id
    async fn get_offer(&self, mint_request_id: &Uuid) -> Result<Option<MintOffer>>;
    /// Resets the mint quote state for the given bill - DEV MODE ONLY
    async fn dev_mode_reset_for_bill(&self, bill_id: &BillId) -> Result<()>;
}
