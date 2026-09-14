use crate::external;
use bcr_ebill_core::{
    application::ValidationError,
    protocol::{Name, ProtocolValidationError, crypto},
};

mod block_transport;
pub mod chain_keys;
mod contact_transport;
mod notification_transport;
pub mod restore;
mod transport;
pub mod transport_client;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use bcr_common::core::NodeId;
use bcr_ebill_core::protocol::crypto::BcrKeys;
use nostr::{
    nips::{nip01::Metadata, nip19::ToBech32},
    types::RelayUrl,
};
use std::time::Duration;

pub use block_transport::BlockTransportServiceApi;
#[cfg(test)]
pub use block_transport::MockBlockTransportServiceApi;

pub use contact_transport::ContactTransportServiceApi;
#[cfg(test)]
pub use contact_transport::MockContactTransportServiceApi;

#[cfg(test)]
pub use notification_transport::MockNotificationTransportServiceApi;
pub use notification_transport::NotificationTransportServiceApi;

#[cfg(test)]
pub use transport::MockTransportServiceApi;
pub use transport::TransportServiceApi;

#[cfg(test)]
pub use transport_client::MockTransportClientApi;
pub use transport_client::TransportClientApi;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Errors stemming from the transport layer that are Network related
    #[error("Network error: {0}")]
    Network(String),

    /// Errors layer that are serialization related, serde will be auto transformed
    #[error("Message serialization error: {0}")]
    Message(String),

    /// Errors that are storage related
    #[error("Persistence error: {0}")]
    Persistence(String),

    /// Errors that are related to a blockchain
    #[error("BlockChain error: {0}")]
    Blockchain(String),

    /// Errors that are related to crypto (keys, encryption, etc.)
    #[error("Crypto error: {0}")]
    Crypto(String),

    /// errors that stem from validation in core
    #[error("Validation Error: {0}")]
    Validation(#[from] ValidationError),

    #[error("External API error: {0}")]
    ExternalApi(#[from] external::Error),

    /// errors if something couldn't be found
    #[error("not found")]
    NotFound,
}

impl From<ProtocolValidationError> for Error {
    fn from(value: ProtocolValidationError) -> Self {
        Self::Validation(ValidationError::Protocol(value))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Message(format!("Failed to serialize/unserialize json message: {e}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Message(format!(
            "Failed to serialize/unserialize borsh message: {e}"
        ))
    }
}

impl From<crypto::Error> for Error {
    fn from(e: crypto::Error) -> Self {
        Error::Crypto(format!("Failed crypto operation: {e}"))
    }
}

impl From<bitcoin::base58::InvalidCharacterError> for Error {
    fn from(e: bitcoin::base58::InvalidCharacterError) -> Self {
        Error::Crypto(format!("Failed base58 operation: {e}"))
    }
}

// Convert ProtocolError to notification_service Error
impl From<bcr_ebill_core::protocol::ProtocolError> for Error {
    fn from(e: bcr_ebill_core::protocol::ProtocolError) -> Self {
        Error::Message(e.to_string())
    }
}

/// A container for collecting contact data from nostr
#[derive(Debug, Clone)]
pub struct NostrContactData {
    pub metadata: Metadata,
    pub relays: Vec<RelayUrl>,
    pub blossom_servers: Vec<url::Url>,
}

impl NostrContactData {
    pub fn new(
        name: &Name,
        relays: Vec<url::Url>,
        blossom_servers: Vec<url::Url>,
        bcr_data: BcrMetadata,
    ) -> Self {
        // At some point we might want to add more metadata like payment info
        let mut metadata = Metadata::new()
            .name(name.as_str())
            .display_name(name.as_str());
        if let Ok(custom) = serde_json::to_value(bcr_data) {
            metadata = metadata.custom_field("bcr", custom);
        }

        Self {
            metadata,
            relays: relays
                .into_iter()
                .filter_map(|r| RelayUrl::parse(r.as_ref()).ok())
                .collect(),
            blossom_servers,
        }
    }

    pub fn get_bcr_metadata(&self) -> Option<BcrMetadata> {
        self.metadata
            .custom
            .get("bcr")
            .cloned()
            .and_then(|c| serde_json::from_value(c).ok())
    }
}

/// Our custom data on nostr Metadata messages
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BcrMetadata {
    /// Encrypted private contact data for sharing with trusted contacts
    pub contact_data: String,
}

#[derive(Clone, Debug)]
pub struct NostrConfig {
    pub keys: BcrKeys,
    pub relays: Vec<url::Url>,
    pub blossom_servers: Vec<url::Url>,
    pub default_timeout: Duration,
    pub is_primary: bool,
    pub node_id: NodeId,
    /// Number of relay acknowledgements required before an optimistic broadcast
    /// returns to the caller. The remaining relays continue publishing in the background.
    pub relay_ack_threshold: usize,
}

impl NostrConfig {
    pub fn new(
        keys: BcrKeys,
        relays: Vec<url::Url>,
        blossom_servers: Vec<url::Url>,
        is_primary: bool,
        node_id: NodeId,
    ) -> Self {
        assert!(!relays.is_empty());
        Self {
            keys,
            relays,
            blossom_servers,
            default_timeout: Duration::from_secs(20),
            is_primary,
            node_id,
            relay_ack_threshold: 1,
        }
    }

    pub fn with_relay_ack_threshold(mut self, threshold: usize) -> Self {
        self.relay_ack_threshold = threshold.max(1);
        self
    }

    pub fn get_npub(&self) -> String {
        self.keys
            .get_nostr_keys()
            .public_key()
            .to_bech32()
            .expect("checked conversion")
    }

    pub fn get_relay(&self) -> url::Url {
        self.relays[0].clone()
    }
}
