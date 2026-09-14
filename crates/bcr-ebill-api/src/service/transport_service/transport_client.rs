use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{
    application::ServiceTraitBounds, protocol::Timestamp, protocol::blockchain::BlockchainType,
    protocol::blockchain::bill::participant::BillParticipant, protocol::crypto::BcrKeys,
    protocol::event::EventEnvelope,
};

#[cfg(test)]
use mockall::automock;

use nostr::{event::Event, filter::Filter, types::RelayUrl};

use super::{NostrContactData, Result};

#[cfg(test)]
impl ServiceTraitBounds for MockTransportClientApi {}

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait TransportClientApi: ServiceTraitBounds {
    /// Sends a private json event to the given recipient.
    async fn send_private_event(
        &self,
        sender_node_id: &NodeId,
        recipient: &BillParticipant,
        event: EventEnvelope,
    ) -> Result<()>;
    /// Builds and signs a public chain event but does NOT broadcast it.
    /// Returns the signed Nostr event for the caller to broadcast and/or queue.
    async fn build_public_chain_event(
        &self,
        sender_node_id: &NodeId,
        id: &str,
        blockchain: BlockchainType,
        block_time: Timestamp,
        event: EventEnvelope,
        previous_event: Option<Event>,
        root_event: Option<Event>,
    ) -> Result<Event>;
    /// Broadcasts a pre-signed event to all configured relays.
    async fn broadcast_event(&self, event: &Event) -> Result<()>;
    /// Broadcasts a pre-signed event to all configured relays, returning as soon as
    /// `min_acks` relays have acknowledged it. Remaining relays continue publishing
    /// in the background.
    async fn broadcast_event_optimistic(&self, event: &Event, min_acks: usize) -> Result<()>;
    /// Returns the configured minimum number of relay acknowledgements required for
    /// an optimistic broadcast to return early.
    fn relay_ack_threshold(&self) -> usize;
    /// Resolves a nostr contact by node id.
    async fn resolve_contact(&self, node_id: &NodeId) -> Result<Option<NostrContactData>>;
    /// Given an id and chain type, tries to resolve the public chain events.
    async fn resolve_public_chain(
        &self,
        id: &str,
        chain_type: BlockchainType,
    ) -> Result<Vec<Event>>;
    /// Adds a new Nostr subscription on the primary client for an added contact
    async fn add_contact_subscription(&self, contact: &NodeId) -> Result<()>;
    /// Resolves private messages matching the filter for all local identities.
    async fn resolve_private_events(&self, filter: Filter) -> Result<Vec<Event>>;

    /// Resolves events matching the given filter. The filter is passed through as-is.
    async fn resolve_events(&self, filter: Filter) -> Result<Vec<Event>>;

    /// Tries to decrypt a private direct message event with any of the local signers.
    /// Returns the recipient NodeId, decrypted EventEnvelope, and sender public key if successful.
    async fn try_decrypt_private_event(
        &self,
        event: &Event,
    ) -> Result<Option<(NodeId, EventEnvelope, nostr::key::PublicKey)>>;

    /// Publishes the metadata (contact info) via the Nostr client for the specified identity
    async fn publish_metadata(
        &self,
        node_id: &NodeId,
        data: &nostr::nips::nip01::Metadata,
    ) -> Result<()>;

    /// Publishes the relay list via the Nostr client for the specified identity
    async fn publish_relay_list(&self, node_id: &NodeId, relays: Vec<RelayUrl>) -> Result<()>;

    /// Publishes the Blossom server list via the Nostr client for the specified identity.
    async fn publish_blossom_server_list(
        &self,
        node_id: &NodeId,
        blossom_servers: Vec<url::Url>,
    ) -> Result<()>;

    /// Publishes file metadata (kind:1063) for the specified file.
    /// This is idempotent - it will only publish if the server URLs have changed.
    async fn publish_file_metadata(
        &self,
        node_id: &NodeId,
        plaintext_hash: &str,
        encrypted_hash: &str,
        server_urls: Vec<url::Url>,
        mime_type: Option<String>,
    ) -> Result<()>;

    /// Opens the connection(s) to the underlying network. This can be called multiple times and
    /// will only open the connection once.
    async fn connect(&self) -> Result<()>;

    /// Adds a new identity (company keys) to the multi-identity client
    /// This will also add a subscription for direct messages to this identity
    async fn add_identity(&self, node_id: NodeId, keys: BcrKeys) -> Result<()>;

    /// Check if this client has a local signer for the given node_id
    fn has_local_signer(&self, node_id: &NodeId) -> bool;

    /// Syncs historical events to newly added relays.
    /// This is called by the background job runner.
    /// Returns early if sync is already in progress.
    async fn sync_relays(&self) -> Result<()>;

    /// Retries events that failed to sync.
    /// This is called by the background job runner.
    async fn retry_failed_syncs(&self) -> Result<()>;

    /// Queries historical file metadata (kind:1063) events for a given file hash.
    /// Returns events that contain server URLs for the file.
    async fn query_file_metadata_events(
        &self,
        file_hash: &str,
        nostr_hash: &str,
    ) -> Result<Vec<Event>>;
}
