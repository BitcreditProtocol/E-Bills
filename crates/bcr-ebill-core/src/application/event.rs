use bcr_common::core::NodeId;
use bitcoin::secp256k1::SecretKey;
use borsh::{BorshDeserialize, BorshSerialize};

/// Event payload when keys for contact details are shared. This is used for both personal identity
/// and company. The shared keys are derived from the private key of the identity or company and
/// all the recipients to decrypt the private Nostr profile data that is stored on relays.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct ContactShareEvent {
    /// The node id of the contact that is shared
    pub node_id: NodeId,
    /// The private key of the contact that is shared
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_privkey",
        deserialize_with = "crate::protocol::serialization::deserialize_privkey"
    )]
    pub private_key: SecretKey,
    /// The pending share ID from the sender's side. When User A shares with User B, this
    /// contains the ID of the outgoing pending share that User A created. User B stores this
    /// ID in their incoming pending share, and when User B shares back, they pass this ID as
    /// share_back_pending_id so User A can auto-accept.
    pub initial_share_id: String,
    /// Optional pending share ID. When present, this is the ID of the pending share from the
    /// original share that this contact share is responding to. If the recipient finds a matching
    /// outgoing pending share with this ID, they will auto-accept this contact instead of creating
    /// a new pending share.
    pub share_back_pending_id: Option<String>,
}
