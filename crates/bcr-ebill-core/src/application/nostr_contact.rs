use std::collections::BTreeSet;

use bcr_common::core::NodeId;
use bitcoin::secp256k1::SecretKey;
use serde::{Deserialize, Serialize};

use crate::{
    application::{ValidationError, contact::Contact},
    protocol::{Name, blockchain::bill::ContactType},
};

/// Make key type clear
pub type NostrPublicKey = nostr::key::PublicKey;

/// Data we need to communicate with a Nostr contact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrContact {
    /// The node id's npub, acts as the primary key.
    pub npub: NostrPublicKey,
    /// The node id of this contact
    pub node_id: NodeId,
    /// The Nostr name of the contact as retrieved via Nostr metadata.
    pub name: Option<Name>,
    /// The relays we found for this contact either from a message or the result of a relay list
    /// query.
    pub relays: Vec<url::Url>,
    /// The Blossom servers we found for this contact from the user server list or relay fallback.
    #[serde(default)]
    pub blossom_servers: Vec<url::Url>,
    /// The trust level we assign to this contact.
    pub trust_level: TrustLevel,
    /// The handshake status with this contact.
    pub handshake_status: HandshakeStatus,
    /// The keys to decrypt private nostr contact details.
    pub contact_private_key: Option<SecretKey>,
    /// Optional mint URL for notifications
    pub mint_url: Option<url::Url>,
}

impl NostrContact {
    /// Creates a new Nostr contact from a contact. This is used when we have a contact and want to
    /// create the Nostr contact from it. Handshake is set to complete and we trust the contact.
    pub fn from_contact(
        contact: &Contact,
        private_key: Option<SecretKey>,
    ) -> Result<Self, ValidationError> {
        let npub = contact.node_id.npub();
        Ok(Self {
            npub,
            node_id: contact.node_id.clone(),
            name: Some(contact.name.clone()),
            relays: contact.nostr_relays.clone(),
            blossom_servers: vec![],
            trust_level: TrustLevel::Trusted,
            handshake_status: HandshakeStatus::Added,
            contact_private_key: private_key,
            mint_url: contact.mint_url.clone(),
        })
    }

    /// Merges contact data into a nostr contact. This assumes at that point the handskake is
    /// complete and we trust the contact.
    pub fn merge_contact(&self, contact: &Contact, private_key: Option<SecretKey>) -> Self {
        let mut relays: BTreeSet<url::Url> = BTreeSet::from_iter(self.relays.clone());
        relays.extend(contact.nostr_relays.clone());
        Self {
            npub: self.npub,
            node_id: self.node_id.clone(),
            name: Some(contact.name.clone()),
            relays: relays.into_iter().collect(),
            blossom_servers: self.blossom_servers.clone(),
            trust_level: TrustLevel::Trusted,
            handshake_status: HandshakeStatus::Added,
            contact_private_key: private_key.or(self.contact_private_key),
            mint_url: contact.mint_url.clone().or(self.mint_url.clone()),
        }
    }

    /// Returns a lightweight version of the contact if all required data is present.
    pub fn into_contact(self, t: Option<ContactType>) -> Option<Contact> {
        if let Some(name) = self.name {
            Some(Contact {
                node_id: self.node_id,
                t: t.unwrap_or(ContactType::Anon),
                name,
                email: None,
                postal_address: None,
                date_of_birth_or_registration: None,
                country_of_birth_or_registration: None,
                city_of_birth_or_registration: None,
                identification_number: None,
                avatar_file: None,
                proof_document_file: None,
                nostr_relays: self.relays,
                is_logical: true,
                mint_url: self.mint_url,
            })
        } else {
            None
        }
    }
}

/// Trust level we assign for a Nostr contact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustLevel {
    /// No trust at all. We don't know this contact and we don't trust them.
    None,
    /// We encountered this contact in a bill so someone we trust trusted them.
    Participant,
    /// We have done a successful contact handshake with this contact and created a real contact
    /// from it.
    Trusted,
    /// A contact we actively banned from communicating with us.
    Banned,
}

/// Handshake is optional but requires some status tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeStatus {
    /// The contact is not yet in the handshake process.
    None,
    /// The contact is in the handshake process.
    InProgress,
    /// The contact has been added to our contacts.
    Added,
}
