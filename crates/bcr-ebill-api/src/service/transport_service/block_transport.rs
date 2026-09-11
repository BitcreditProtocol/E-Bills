use super::Result;
use crate::service::transport_service::ResyncMode;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::{
        blockchain::bill::BillBlock,
        event::{BillChainEvent, CompanyChainEvent, IdentityChainEvent},
    },
};

#[cfg(test)]
use mockall::automock;

/// Methods required for all block propagations and chain re-syncs
#[allow(dead_code)]
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait BlockTransportServiceApi: ServiceTraitBounds {
    /// Sent when an identity chain is created or updated
    async fn send_identity_chain_events(&self, events: IdentityChainEvent) -> Result<()>;
    /// Sent when a company chain is created or updated
    async fn send_company_chain_events(&self, events: CompanyChainEvent) -> Result<()>;
    /// Sent when: A bill chain is created or updated
    async fn send_bill_chain_events(&self, events: BillChainEvent) -> Result<()>;
    /// Resync bill chain. If `from_nostr` is true, fetches missing blocks from Nostr first.
    /// If false, only invalidates the local cache.
    async fn resync_bill_chain(
        &self,
        bill_id: &BillId,
        from_nostr: bool,
        mode: ResyncMode,
    ) -> Result<()>;
    /// Resync company chain
    async fn resync_company_chain(&self, company_id: &NodeId) -> Result<()>;
    /// Resync identity chain
    async fn resync_identity_chain(&self) -> Result<()>;
    /// Validates that the given list of blocks exist in the resolved chain from Nostr
    async fn validate_bill_blocks_exist_on_nostr_chain(
        &self,
        bill_id: &BillId,
        blocks: &[BillBlock],
    ) -> Result<bool>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockBlockTransportServiceApi {}
