use super::{
    Error, Result,
    surreal::{Bindings, SurrealWrapper},
};
use crate::{
    constants::{
        DB_CONTACT_SHARE_DIRECTION, DB_HANDSHAKE_STATUS, DB_ID, DB_NODE_ID, DB_RECEIVER_NODE_ID,
        DB_SEARCH_TERM, DB_TABLE, DB_TRUST_LEVEL,
    },
    nostr::{NostrStoreApi, PendingContactShare, RelaySyncStatus, ShareDirection, SyncStatus},
};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    application::contact::Contact,
    application::nostr_contact::{HandshakeStatus, NostrContact, NostrPublicKey, TrustLevel},
    protocol::Name,
    protocol::SecretKey,
    protocol::Timestamp,
};
use log::error;
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;

#[derive(Clone)]
pub struct SurrealNostrStore {
    db: SurrealWrapper,
}

impl SurrealNostrStore {
    const TABLE: &'static str = "nostr_contact";
    const PENDING_SHARE_TABLE: &'static str = "pending_contact_share";
    const RELAY_SYNC_TABLE: &'static str = "relay_sync_status";
    const RELAY_RETRY_TABLE: &'static str = "relay_sync_retry";

    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }

    fn thing_id(id: &str) -> Thing {
        Thing::from((Self::TABLE.to_owned(), id.to_owned()))
    }
}

impl ServiceTraitBounds for SurrealNostrStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NostrStoreApi for SurrealNostrStore {
    /// Find a Nostr contact by the node id. This is the primary key for the contact.
    async fn by_node_id(&self, node_id: &NodeId) -> Result<Option<NostrContact>> {
        let npub = node_id.npub();
        self.by_npub(&npub).await
    }

    async fn by_node_ids(&self, node_ids: Vec<NodeId>) -> Result<Vec<NostrContact>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_NODE_ID, node_ids)?;
        let query =
            format!("SELECT * from type::table(${DB_TABLE}) WHERE {DB_NODE_ID} IN ${DB_NODE_ID}");
        let result: Vec<NostrContactDb> = self.db.query(&query, bindings).await?;
        let values = result
            .into_iter()
            .map(|c| c.to_owned().try_into().ok())
            .collect::<Option<Vec<NostrContact>>>();
        Ok(values.unwrap_or_default())
    }

    /// Get all Nostr contacts from the store.
    async fn get_all(&self) -> Result<Vec<NostrContact>> {
        let result: Vec<NostrContactDb> = self.db.select_all(Self::TABLE).await?;
        let values = result
            .into_iter()
            .filter_map(|c| match c.try_into() {
                Ok(v) => Some(v),
                Err(e) => {
                    error!("Failed to convert NostrContactDb to NostrContact: {e}");
                    None
                }
            })
            .collect::<Vec<NostrContact>>();
        Ok(values)
    }

    /// Find a Nostr contact by the npub. This is the public Nostr key of the contact.
    async fn by_npub(&self, npub: &NostrPublicKey) -> Result<Option<NostrContact>> {
        let result: Option<NostrContactDb> = self.db.select_one(Self::TABLE, npub.to_hex()).await?;
        let value = result.and_then(|v| v.to_owned().try_into().ok());
        Ok(value)
    }
    /// Creates a new or updates an existing Nostr contact.
    async fn upsert(&self, data: &NostrContact) -> Result<()> {
        let db_data: NostrContactDb = data.clone().into();
        let _: Option<NostrContactDb> = self
            .db
            .upsert(Self::TABLE, data.npub.to_hex(), db_data)
            .await?;
        Ok(())
    }
    /// Delete an Nostr contact. This will remove the contact from the store.
    async fn delete(&self, node_id: &NodeId) -> Result<()> {
        let npub = node_id.npub().to_hex();
        let _: Option<NostrContactDb> = self.db.delete(Self::TABLE, npub.to_owned()).await?;
        Ok(())
    }
    /// Sets a new handshake status for the contact. This is used to track the handshake process.
    async fn set_handshake_status(&self, node_id: &NodeId, status: HandshakeStatus) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_HANDSHAKE_STATUS, status)?;
        bindings.add(DB_ID, Self::thing_id(&node_id.npub().to_hex()))?;
        self.db
            .query_check(&update_field_query(DB_HANDSHAKE_STATUS), bindings)
            .await?;
        Ok(())
    }
    /// Sets a new trust level for the contact. This is used to track the trust level of the
    /// contact.
    async fn set_trust_level(&self, node_id: &NodeId, trust_level: TrustLevel) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_TRUST_LEVEL, trust_level)?;
        bindings.add(DB_ID, Self::thing_id(&node_id.npub().to_hex()))?;
        self.db
            .query_check(&update_field_query(DB_TRUST_LEVEL), bindings)
            .await?;
        Ok(())
    }

    /// Returns all npubs that have a trust level higher than or equal to the given level.
    async fn get_npubs(&self, levels: Vec<TrustLevel>) -> Result<Vec<NostrPublicKey>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_TRUST_LEVEL, levels)?;
        let query = format!(
            "SELECT * from type::table(${DB_TABLE}) where {DB_TRUST_LEVEL} IN ${DB_TRUST_LEVEL}"
        );
        let result: Vec<NostrContactDb> = self.db.query(&query, bindings).await?;
        let keys = result
            .into_iter()
            .filter_map(|c| NostrPublicKey::parse(&c.id.id.to_raw()).ok())
            .collect::<Vec<NostrPublicKey>>();
        Ok(keys)
    }

    async fn search(
        &self,
        search_term: &str,
        levels: Vec<TrustLevel>,
    ) -> Result<Vec<NostrContact>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_TRUST_LEVEL, levels)?;
        bindings.add(DB_SEARCH_TERM, search_term.to_lowercase().to_owned())?;
        let query = format!(
            "SELECT * from type::table(${DB_TABLE}) WHERE {DB_TRUST_LEVEL} IN ${DB_TRUST_LEVEL} AND string::lowercase(name) CONTAINS ${DB_SEARCH_TERM}"
        );
        let result: Vec<NostrContactDb> = self.db.query(&query, bindings).await?;
        let values = result
            .into_iter()
            .map(|c| c.to_owned().try_into().ok())
            .collect::<Option<Vec<NostrContact>>>();
        Ok(values.unwrap_or_default())
    }

    // Pending contact share methods
    async fn add_pending_share(&self, pending_share: PendingContactShare) -> Result<()> {
        let db_data: PendingContactShareDb = pending_share.clone().into();
        let _: Option<PendingContactShareDb> = self
            .db
            .upsert(Self::PENDING_SHARE_TABLE, pending_share.id.clone(), db_data)
            .await?;
        Ok(())
    }

    async fn get_pending_share(&self, id: &str) -> Result<Option<PendingContactShare>> {
        let result: Option<PendingContactShareDb> = self
            .db
            .select_one(Self::PENDING_SHARE_TABLE, id.to_string())
            .await?;
        let value = result.and_then(|v| v.to_owned().try_into().ok());
        Ok(value)
    }

    async fn get_pending_share_by_private_key(
        &self,
        private_key: &SecretKey,
    ) -> Result<Option<PendingContactShare>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::PENDING_SHARE_TABLE)?;
        bindings.add("contact_private_key", private_key.to_owned())?;
        let query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE contact_private_key = $contact_private_key LIMIT 1"
        );
        let result: Vec<PendingContactShareDb> = self.db.query(&query, bindings).await?;
        let value = result.into_iter().next().and_then(|v| v.try_into().ok());
        Ok(value)
    }

    async fn list_pending_shares_by_receiver(
        &self,
        receiver_node_id: &NodeId,
    ) -> Result<Vec<PendingContactShare>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::PENDING_SHARE_TABLE)?;
        bindings.add(DB_RECEIVER_NODE_ID, receiver_node_id.to_owned())?;
        let query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE {DB_RECEIVER_NODE_ID} = ${DB_RECEIVER_NODE_ID} ORDER BY received_at DESC"
        );
        let result: Vec<PendingContactShareDb> = self.db.query(&query, bindings).await?;
        let values = result
            .into_iter()
            .map(|c| c.to_owned().try_into().ok())
            .collect::<Option<Vec<PendingContactShare>>>();
        Ok(values.unwrap_or_default())
    }

    async fn list_pending_shares_by_receiver_and_direction(
        &self,
        receiver_node_id: &NodeId,
        direction: ShareDirection,
    ) -> Result<Vec<PendingContactShare>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::PENDING_SHARE_TABLE)?;
        bindings.add(DB_RECEIVER_NODE_ID, receiver_node_id.to_owned())?;
        bindings.add(DB_CONTACT_SHARE_DIRECTION, direction)?;
        let query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE {DB_RECEIVER_NODE_ID} = ${DB_RECEIVER_NODE_ID} AND ${DB_CONTACT_SHARE_DIRECTION} = $direction ORDER BY received_at DESC"
        );
        let result: Vec<PendingContactShareDb> = self.db.query(&query, bindings).await?;
        let values = result
            .into_iter()
            .map(|c| c.to_owned().try_into().ok())
            .collect::<Option<Vec<PendingContactShare>>>();
        Ok(values.unwrap_or_default())
    }

    async fn delete_pending_share(&self, id: &str) -> Result<()> {
        let _: Option<PendingContactShareDb> = self
            .db
            .delete(Self::PENDING_SHARE_TABLE, id.to_owned())
            .await?;
        Ok(())
    }

    async fn pending_share_exists_for_node_and_receiver(
        &self,
        node_id: &NodeId,
        receiver_node_id: &NodeId,
        direction: ShareDirection,
    ) -> Result<bool> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::PENDING_SHARE_TABLE)?;
        bindings.add(DB_NODE_ID, node_id.to_owned())?;
        bindings.add(DB_RECEIVER_NODE_ID, receiver_node_id.to_owned())?;
        bindings.add(DB_CONTACT_SHARE_DIRECTION, direction)?;
        let query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE {DB_NODE_ID} = ${DB_NODE_ID} AND {DB_RECEIVER_NODE_ID} = ${DB_RECEIVER_NODE_ID} AND {DB_CONTACT_SHARE_DIRECTION} = ${DB_CONTACT_SHARE_DIRECTION} LIMIT 1"
        );
        let result: Vec<PendingContactShareDb> = self.db.query(&query, bindings).await?;
        Ok(!result.is_empty())
    }

    // === Relay Sync Status Methods ===

    async fn get_pending_relays(&self) -> Result<Vec<url::Url>> {
        let statuses: Vec<RelaySyncStatusDb> = self.db.select_all(Self::RELAY_SYNC_TABLE).await?;
        let pending: Vec<url::Url> = statuses
            .into_iter()
            .filter(|s| {
                matches!(
                    s.sync_status,
                    SyncStatus::Pending | SyncStatus::InProgress | SyncStatus::Failed
                )
            })
            .filter_map(|s| url::Url::parse(&s.relay_url).ok())
            .collect();
        Ok(pending)
    }

    async fn get_relay_sync_status(&self, relay: &url::Url) -> Result<Option<RelaySyncStatus>> {
        let result: Option<RelaySyncStatusDb> = self
            .db
            .select_one(Self::RELAY_SYNC_TABLE, relay.to_string())
            .await?;
        match result {
            Some(db) => Ok(Some(db.try_into()?)),
            None => Ok(None),
        }
    }

    async fn update_relay_sync_status(&self, relay: &url::Url, status: SyncStatus) -> Result<()> {
        // Get existing record or create new one
        let existing = self.get_relay_sync_status(relay).await?;
        let updated = match existing {
            Some(mut s) => {
                s.sync_status = status.clone();
                if matches!(status, SyncStatus::Failed) {
                    // Keep last_error if it exists
                } else if matches!(status, SyncStatus::Completed) {
                    s.last_error = None;
                }
                s
            }
            None => RelaySyncStatus {
                relay_url: relay.clone(),
                last_seen_in_config: Timestamp::now(),
                sync_status: status,
                events_synced: 0,
                last_synced_timestamp: None,
                last_error: None,
            },
        };
        let db_data: RelaySyncStatusDb = updated.into();
        let _: Option<RelaySyncStatusDb> = self
            .db
            .upsert(Self::RELAY_SYNC_TABLE, relay.to_string(), db_data)
            .await?;
        Ok(())
    }

    async fn update_relay_sync_progress(
        &self,
        relay: &url::Url,
        timestamp: Timestamp,
    ) -> Result<()> {
        let existing = self.get_relay_sync_status(relay).await?;
        let updated = match existing {
            Some(mut s) => {
                s.events_synced += 1;
                s.last_synced_timestamp = Some(timestamp);
                s
            }
            None => {
                return Err(Error::Persistence(format!(
                    "Relay sync status not found for {}",
                    relay
                )));
            }
        };
        let db_data: RelaySyncStatusDb = updated.into();
        let _: Option<RelaySyncStatusDb> = self
            .db
            .upsert(Self::RELAY_SYNC_TABLE, relay.to_string(), db_data)
            .await?;
        Ok(())
    }

    async fn update_relay_last_seen(&self, relay: &url::Url, timestamp: Timestamp) -> Result<()> {
        let existing = self.get_relay_sync_status(relay).await?;
        let updated = match existing {
            Some(mut s) => {
                s.last_seen_in_config = timestamp;
                s
            }
            None => RelaySyncStatus {
                relay_url: relay.clone(),
                last_seen_in_config: timestamp,
                sync_status: SyncStatus::Pending,
                events_synced: 0,
                last_synced_timestamp: None,
                last_error: None,
            },
        };
        let db_data: RelaySyncStatusDb = updated.into();
        let _: Option<RelaySyncStatusDb> = self
            .db
            .upsert(Self::RELAY_SYNC_TABLE, relay.to_string(), db_data)
            .await?;
        Ok(())
    }

    // === Relay Sync Retry Queue Methods ===

    async fn add_failed_relay_sync(
        &self,
        relay: &url::Url,
        event: nostr::event::Event,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let retry = RelaySyncRetryDb {
            id: Thing::from((Self::RELAY_RETRY_TABLE.to_string(), id.clone())),
            relay_url: relay.to_string(),
            event,
            retry_count: 0,
            created_at: Timestamp::now(),
            last_retry_at: None,
        };
        let _: Option<RelaySyncRetryDb> =
            self.db.upsert(Self::RELAY_RETRY_TABLE, id, retry).await?;
        Ok(())
    }

    async fn get_pending_relay_retries(
        &self,
        relay: &url::Url,
        limit: usize,
    ) -> Result<Vec<nostr::event::Event>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::RELAY_RETRY_TABLE)?;
        bindings.add("relay_url", relay.to_string())?;
        bindings.add("limit", limit as i64)?;

        let query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE relay_url = $relay_url LIMIT $limit"
        );

        let retries: Vec<RelaySyncRetryDb> = self.db.query(&query, bindings).await?;
        let events: Vec<nostr::event::Event> = retries.into_iter().map(|r| r.event).collect();
        Ok(events)
    }

    async fn mark_relay_retry_success(&self, relay: &url::Url, event_id: &str) -> Result<()> {
        // Query for matching records - SurrealDB supports querying nested fields
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::RELAY_RETRY_TABLE)?;
        bindings.add("relay_url", relay.to_string())?;
        bindings.add("event_id", event_id.to_string())?;

        let query = format!(
            "DELETE FROM type::table(${DB_TABLE}) WHERE relay_url = $relay_url AND event.id == $event_id"
        );

        let _: Vec<RelaySyncRetryDb> = self.db.query(&query, bindings).await?;
        Ok(())
    }

    async fn mark_relay_retry_failed(
        &self,
        relay: &url::Url,
        event_id: &str,
        max_retries: usize,
    ) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::RELAY_RETRY_TABLE)?;
        bindings.add("relay_url", relay.to_string())?;
        bindings.add("event_id", event_id.to_string())?;
        bindings.add("max_retries", max_retries as i64)?;
        bindings.add("now", Timestamp::now())?;

        let select_query = format!(
            "SELECT * FROM type::table(${DB_TABLE}) WHERE relay_url = $relay_url AND event.id == $event_id"
        );
        let retries: Vec<RelaySyncRetryDb> = self.db.query(&select_query, bindings.clone()).await?;

        if let Some(retry) = retries.first() {
            if retry.retry_count >= max_retries {
                // Max retries exceeded, delete
                let delete_query = format!(
                    "DELETE FROM type::table(${DB_TABLE}) WHERE relay_url = $relay_url AND event.id == $event_id"
                );
                let _: Vec<RelaySyncRetryDb> = self.db.query(&delete_query, bindings).await?;
            } else {
                // Increment retry count
                let update_query = format!(
                    "UPDATE type::table(${DB_TABLE}) SET retry_count += 1, last_retry_at = $now WHERE relay_url = $relay_url AND event.id == $event_id"
                );
                let _: Vec<RelaySyncRetryDb> = self.db.query(&update_query, bindings).await?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelaySyncStatusDb {
    id: Thing,
    relay_url: String,
    last_seen_in_config: Timestamp,
    sync_status: SyncStatus,
    events_synced: usize,
    last_synced_timestamp: Option<Timestamp>,
    last_error: Option<String>,
}

impl From<RelaySyncStatus> for RelaySyncStatusDb {
    fn from(value: RelaySyncStatus) -> Self {
        Self {
            id: Thing::from((
                SurrealNostrStore::RELAY_SYNC_TABLE.to_string(),
                value.relay_url.to_string(),
            )),
            relay_url: value.relay_url.to_string(),
            last_seen_in_config: value.last_seen_in_config,
            sync_status: value.sync_status,
            events_synced: value.events_synced,
            last_synced_timestamp: value.last_synced_timestamp,
            last_error: value.last_error,
        }
    }
}

impl TryFrom<RelaySyncStatusDb> for RelaySyncStatus {
    type Error = Error;

    fn try_from(value: RelaySyncStatusDb) -> Result<Self> {
        Ok(Self {
            relay_url: url::Url::parse(&value.relay_url)
                .map_err(|_| Error::Persistence("Invalid relay URL".to_string()))?,
            last_seen_in_config: value.last_seen_in_config,
            sync_status: value.sync_status,
            events_synced: value.events_synced,
            last_synced_timestamp: value.last_synced_timestamp,
            last_error: value.last_error,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelaySyncRetryDb {
    id: Thing,
    relay_url: String,
    event: nostr::event::Event,
    retry_count: usize,
    created_at: Timestamp,
    last_retry_at: Option<Timestamp>,
}

fn update_field_query(field_name: &str) -> String {
    format!(
        "UPDATE type::table(${DB_TABLE}) SET {field_name} = ${field_name} WHERE {DB_ID} = ${DB_ID}"
    )
}

/// Data we need to communicate with a Nostr contact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrContactDb {
    /// Our node id. This is the node id and acts as the primary key.
    pub id: Thing,
    /// The node id of this contact
    pub node_id: NodeId,
    /// The Nostr name of the contact as retreived via Nostr metadata.
    pub name: Option<Name>,
    /// The relays we found for this contact either from a message or the result of a relay list
    /// query.
    pub relays: Vec<url::Url>,
    #[serde(default)]
    pub blossom_servers: Vec<url::Url>,
    /// The trust level we assign to this contact.
    pub trust_level: TrustLevel,
    /// The handshake status with this contact.
    pub handshake_status: HandshakeStatus,
    /// The keys to decrypt private contact details.
    pub contact_private_key: Option<SecretKey>,
    /// Optional mint URL for notifications
    pub mint_url: Option<url::Url>,
}

impl From<NostrContact> for NostrContactDb {
    fn from(contact: NostrContact) -> Self {
        Self {
            id: Thing::from((SurrealNostrStore::TABLE.to_owned(), contact.npub.to_hex())),
            node_id: contact.node_id,
            name: contact.name,
            relays: contact.relays,
            blossom_servers: contact.blossom_servers,
            trust_level: contact.trust_level,
            handshake_status: contact.handshake_status,
            contact_private_key: contact.contact_private_key,
            mint_url: contact.mint_url,
        }
    }
}

impl TryFrom<NostrContactDb> for NostrContact {
    type Error = Error;
    fn try_from(db: NostrContactDb) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            npub: NostrPublicKey::parse(&db.id.id.to_raw()).map_err(|_| Error::EncodingError)?,
            node_id: db.node_id,
            name: db.name,
            relays: db.relays,
            blossom_servers: db.blossom_servers,
            trust_level: db.trust_level,
            handshake_status: db.handshake_status,
            contact_private_key: db.contact_private_key,
            mint_url: db.mint_url,
        })
    }
}

/// Database representation of a pending contact share
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingContactShareDb {
    pub id: Thing,
    pub node_id: NodeId,
    pub contact: Contact,
    pub sender_node_id: NodeId,
    pub contact_private_key: SecretKey,
    pub receiver_node_id: NodeId,
    pub received_at: Timestamp,
    pub direction: ShareDirection,
    pub initial_share_id: Option<String>,
}

impl From<PendingContactShare> for PendingContactShareDb {
    fn from(share: PendingContactShare) -> Self {
        Self {
            id: Thing::from((SurrealNostrStore::PENDING_SHARE_TABLE.to_owned(), share.id)),
            node_id: share.node_id,
            contact: share.contact,
            sender_node_id: share.sender_node_id,
            contact_private_key: share.contact_private_key,
            receiver_node_id: share.receiver_node_id,
            received_at: share.received_at,
            direction: share.direction,
            initial_share_id: share.initial_share_id,
        }
    }
}

impl TryFrom<PendingContactShareDb> for PendingContactShare {
    type Error = Error;
    fn try_from(db: PendingContactShareDb) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            id: db.id.id.to_raw(),
            node_id: db.node_id,
            contact: db.contact,
            sender_node_id: db.sender_node_id,
            contact_private_key: db.contact_private_key,
            receiver_node_id: db.receiver_node_id,
            received_at: db.received_at,
            direction: db.direction,
            initial_share_id: db.initial_share_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use bcr_ebill_core::protocol::crypto::BcrKeys;
    use nostr::event::FinalizeEvent;

    use super::*;
    use crate::db::get_memory_db;

    #[tokio::test]
    async fn test_upsert_and_retrieve_by_node_id() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Retrieve the contact by node_id
        let retrieved = store
            .by_node_id(&node_id)
            .await
            .expect("Failed to retrieve contact")
            .expect("Contact not found");

        assert_eq!(retrieved.npub, contact.npub);
        assert_eq!(retrieved.name, contact.name);
        assert_eq!(retrieved.relays, contact.relays);
        assert_eq!(retrieved.trust_level, contact.trust_level);
        assert_eq!(retrieved.handshake_status, contact.handshake_status);
    }

    #[tokio::test]
    async fn test_upsert_and_retrieve_by_npub() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Retrieve the contact by node_id
        let retrieved = store
            .by_npub(&node_id.npub())
            .await
            .expect("Failed to retrieve contact by npub")
            .expect("Contact by npub not found");

        assert_eq!(retrieved.npub, contact.npub);
    }

    #[tokio::test]
    async fn test_delete_contact() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Delete the contact
        store
            .delete(&node_id)
            .await
            .expect("Failed to delete contact");

        // Try to retrieve the contact
        let retrieved = store
            .by_node_id(&node_id)
            .await
            .expect("Failed to retrieve contact");
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_set_handshake_status() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Update handshake status
        store
            .set_handshake_status(&node_id, HandshakeStatus::InProgress)
            .await
            .expect("Failed to set handshake status");

        // Retrieve the contact and verify the handshake status
        let retrieved = store
            .by_node_id(&node_id)
            .await
            .expect("Failed to retrieve contact")
            .expect("Contact not found");

        assert_eq!(retrieved.handshake_status, HandshakeStatus::InProgress);
    }

    #[tokio::test]
    async fn test_set_trust_level() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Update trust level
        store
            .set_trust_level(&node_id, TrustLevel::Participant)
            .await
            .expect("Failed to set trust level");

        // Retrieve the contact and verify the trust level
        let retrieved = store
            .by_node_id(&node_id)
            .await
            .expect("Failed to retrieve contact")
            .expect("Contact not found");

        assert_eq!(retrieved.trust_level, TrustLevel::Participant);
    }

    #[tokio::test]
    async fn test_get_npubs() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, None);

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Update trust level
        store
            .set_trust_level(&node_id, TrustLevel::Participant)
            .await
            .expect("Failed to set trust level");

        // Retrieve the contact and verify the trust level
        let retrieved = store
            .get_npubs(vec![TrustLevel::Participant])
            .await
            .expect("Failed to retrieve contact");

        assert!(!retrieved.is_empty());
    }

    #[tokio::test]
    async fn test_search() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, Some(Name::new("Albert").unwrap()));

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        // Update trust level
        store
            .set_trust_level(&node_id, TrustLevel::Participant)
            .await
            .expect("Failed to set trust level");

        let keys2 = BcrKeys::new();
        let node_id2 = NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);
        let contact2 = get_test_contact(&node_id2, Some(Name::new("Berta").unwrap()));

        // Upsert the contact
        store
            .upsert(&contact2)
            .await
            .expect("Failed to upsert contact");

        // Update trust level
        store
            .set_trust_level(&node_id2, TrustLevel::Trusted)
            .await
            .expect("Failed to set trust level");

        let keys3 = BcrKeys::new();
        let node_id3 = NodeId::new(keys3.pub_key(), bitcoin::Network::Testnet);
        let contact3 = get_test_contact(&node_id3, Some(Name::new("Bertrand").unwrap()));

        // Upsert the contact
        store
            .upsert(&contact3)
            .await
            .expect("Failed to upsert contact");

        let result = store
            .search(
                "bert",
                vec![
                    TrustLevel::Participant,
                    TrustLevel::Trusted,
                    TrustLevel::None,
                ],
            )
            .await
            .expect("Could not search");
        assert_eq!(result.len(), 3, "Search did not return all trust levels");

        let result = store
            .search("bert", vec![TrustLevel::Participant, TrustLevel::Trusted])
            .await
            .expect("Could not search");
        assert_eq!(result.len(), 2, "Search did not filter by trust levels");

        let result = store
            .search("ALB", vec![TrustLevel::Participant, TrustLevel::Trusted])
            .await
            .expect("Could not search");
        assert_eq!(result.len(), 1, "Search did not filter by name");

        let result = store
            .search("Berta", vec![TrustLevel::Participant, TrustLevel::Trusted])
            .await
            .expect("Could not search");
        assert_eq!(result.len(), 1, "Search did not filter by name");
    }

    #[tokio::test]
    async fn test_by_node_ids() {
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let store = get_store().await;
        let contact = get_test_contact(&node_id, Some(Name::new("Albert").unwrap()));

        // Upsert the contact
        store
            .upsert(&contact)
            .await
            .expect("Failed to upsert contact");

        let keys2 = BcrKeys::new();
        let node_id2 = NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);
        let contact2 = get_test_contact(&node_id2, Some(Name::new("Berta").unwrap()));

        // Upsert the contact
        store
            .upsert(&contact2)
            .await
            .expect("Failed to upsert contact");

        let keys3 = BcrKeys::new();
        let node_id3 = NodeId::new(keys3.pub_key(), bitcoin::Network::Testnet);

        let result = store
            .by_node_ids(vec![node_id, node_id2, node_id3])
            .await
            .expect("Could not find by node ids");
        assert_eq!(
            result.len(),
            2,
            "Find by node ids did not return all expected contacts"
        );
    }

    async fn get_store() -> SurrealNostrStore {
        let mem_db = get_memory_db("test", "nostr_contact")
            .await
            .expect("could not create memory db");
        SurrealNostrStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    fn get_test_contact(node_id: &NodeId, name: Option<Name>) -> NostrContact {
        NostrContact {
            npub: node_id.npub(),
            node_id: node_id.clone(),
            name: name.or(Some(Name::new("contact_name").unwrap())),
            relays: vec![url::Url::parse("ws://localhost:8080").unwrap()],
            blossom_servers: vec![],
            trust_level: TrustLevel::None,
            handshake_status: HandshakeStatus::None,
            contact_private_key: None,
            mint_url: None,
        }
    }

    // === Relay Sync Database Tests ===

    #[tokio::test]
    async fn test_update_relay_last_seen_creates_new_status() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let now = Timestamp::now();

        // Update last_seen for a relay that doesn't exist yet
        store
            .update_relay_last_seen(&relay, now)
            .await
            .expect("Failed to update relay last seen");

        // Should create new status record with Pending status
        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get relay sync status")
            .expect("Status should exist");

        assert_eq!(status.relay_url, relay);
        assert_eq!(status.last_seen_in_config, now);
        assert_eq!(status.sync_status, SyncStatus::Pending);
        assert_eq!(status.events_synced, 0);
        assert!(status.last_synced_timestamp.is_none());
        assert!(status.last_error.is_none());
    }

    #[tokio::test]
    async fn test_update_relay_last_seen_updates_existing() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let initial_time = Timestamp::new(1000).unwrap();
        let later_time = Timestamp::new(2000).unwrap();

        // Create initial status
        store
            .update_relay_last_seen(&relay, initial_time)
            .await
            .expect("Failed to create initial status");

        // Update to later time
        store
            .update_relay_last_seen(&relay, later_time)
            .await
            .expect("Failed to update last seen");

        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get status")
            .expect("Status should exist");

        assert_eq!(status.last_seen_in_config, later_time);
    }

    #[tokio::test]
    async fn test_update_relay_sync_status() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        // Create initial Pending status
        store
            .update_relay_sync_status(&relay, SyncStatus::Pending)
            .await
            .expect("Failed to create status");

        // Update to InProgress
        store
            .update_relay_sync_status(&relay, SyncStatus::InProgress)
            .await
            .expect("Failed to update status");

        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get status")
            .expect("Status should exist");

        assert_eq!(status.sync_status, SyncStatus::InProgress);

        // Update to Completed
        store
            .update_relay_sync_status(&relay, SyncStatus::Completed)
            .await
            .expect("Failed to update to completed");

        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get status")
            .expect("Status should exist");

        assert_eq!(status.sync_status, SyncStatus::Completed);
        assert!(status.last_error.is_none());
    }

    #[tokio::test]
    async fn test_get_pending_relays() {
        let store = get_store().await;
        let relay1 = url::Url::parse("wss://relay1.example.com").unwrap();
        let relay2 = url::Url::parse("wss://relay2.example.com").unwrap();
        let relay3 = url::Url::parse("wss://relay3.example.com").unwrap();
        let relay4 = url::Url::parse("wss://relay4.example.com").unwrap();

        // Create different statuses
        store
            .update_relay_sync_status(&relay1, SyncStatus::Pending)
            .await
            .expect("Failed to create pending");
        store
            .update_relay_sync_status(&relay2, SyncStatus::InProgress)
            .await
            .expect("Failed to create in progress");
        store
            .update_relay_sync_status(&relay3, SyncStatus::Completed)
            .await
            .expect("Failed to create completed");
        store
            .update_relay_sync_status(&relay4, SyncStatus::Failed)
            .await
            .expect("Failed to create failed");

        let pending = store
            .get_pending_relays()
            .await
            .expect("Failed to get pending relays");

        // Should return Pending, InProgress, and Failed but not Completed
        assert_eq!(pending.len(), 3);
        assert!(pending.contains(&relay1));
        assert!(pending.contains(&relay2));
        assert!(pending.contains(&relay4));
        assert!(!pending.contains(&relay3));
    }

    #[tokio::test]
    async fn test_update_relay_sync_progress() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let timestamp1 = Timestamp::new(1000).unwrap();
        let timestamp2 = Timestamp::new(2000).unwrap();

        // Create initial status
        store
            .update_relay_sync_status(&relay, SyncStatus::InProgress)
            .await
            .expect("Failed to create status");

        // Update progress
        store
            .update_relay_sync_progress(&relay, timestamp1)
            .await
            .expect("Failed to update progress");

        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get status")
            .expect("Status should exist");

        assert_eq!(status.events_synced, 1);
        assert_eq!(status.last_synced_timestamp, Some(timestamp1));

        // Update again with later timestamp
        store
            .update_relay_sync_progress(&relay, timestamp2)
            .await
            .expect("Failed to update progress again");

        let status = store
            .get_relay_sync_status(&relay)
            .await
            .expect("Failed to get status")
            .expect("Status should exist");

        assert_eq!(status.events_synced, 2);
        assert_eq!(status.last_synced_timestamp, Some(timestamp2));
    }

    #[tokio::test]
    async fn test_add_and_get_pending_relay_retries() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        // Create test events
        let keys = nostr::key::Keys::generate();
        let event1 = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test 1")
            .finalize(&keys)
            .expect("Failed to sign event");
        let event2 = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test 2")
            .finalize(&keys)
            .expect("Failed to sign event");

        // Add to retry queue
        store
            .add_failed_relay_sync(&relay, event1.clone())
            .await
            .expect("Failed to add retry");
        store
            .add_failed_relay_sync(&relay, event2.clone())
            .await
            .expect("Failed to add retry");

        // Get pending retries
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");

        assert_eq!(retries.len(), 2);
        assert!(retries.iter().any(|e| e.id == event1.id));
        assert!(retries.iter().any(|e| e.id == event2.id));
    }

    #[tokio::test]
    async fn test_mark_relay_retry_success() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        let keys = nostr::key::Keys::generate();
        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&keys)
            .expect("Failed to sign event");

        // Add to retry queue
        store
            .add_failed_relay_sync(&relay, event.clone())
            .await
            .expect("Failed to add retry");

        // Verify it's there
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 1);

        // Mark as success
        store
            .mark_relay_retry_success(&relay, &event.id.to_hex())
            .await
            .expect("Failed to mark success");

        // Should be removed from queue
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 0);
    }

    #[tokio::test]
    async fn test_mark_relay_retry_failed_increments_count() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        let keys = nostr::key::Keys::generate();
        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&keys)
            .expect("Failed to sign event");

        // Add to retry queue
        store
            .add_failed_relay_sync(&relay, event.clone())
            .await
            .expect("Failed to add retry");

        // Mark as failed (max retries = 3)
        store
            .mark_relay_retry_failed(&relay, &event.id.to_hex(), 3)
            .await
            .expect("Failed to mark failed");

        // Should still be in queue (retry_count incremented)
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 1);

        // Mark as failed again
        store
            .mark_relay_retry_failed(&relay, &event.id.to_hex(), 3)
            .await
            .expect("Failed to mark failed");

        // Still in queue
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 1);

        // Mark as failed one more time
        store
            .mark_relay_retry_failed(&relay, &event.id.to_hex(), 3)
            .await
            .expect("Failed to mark failed");

        // Still in queue (retry_count = 3, max = 3, not yet exceeded)
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 1);

        // One more time should remove it (retry_count would be 4 > max 3)
        store
            .mark_relay_retry_failed(&relay, &event.id.to_hex(), 3)
            .await
            .expect("Failed to mark failed");

        // Should be removed
        let retries = store
            .get_pending_relay_retries(&relay, 10)
            .await
            .expect("Failed to get retries");
        assert_eq!(retries.len(), 0);
    }

    #[tokio::test]
    async fn test_get_pending_relay_retries_filters_by_relay() {
        let store = get_store().await;
        let relay1 = url::Url::parse("wss://relay1.example.com").unwrap();
        let relay2 = url::Url::parse("wss://relay2.example.com").unwrap();

        let keys = nostr::key::Keys::generate();
        let event1 = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test 1")
            .finalize(&keys)
            .expect("Failed to sign event");
        let event2 = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test 2")
            .finalize(&keys)
            .expect("Failed to sign event");

        // Add events to different relays
        store
            .add_failed_relay_sync(&relay1, event1.clone())
            .await
            .expect("Failed to add retry");
        store
            .add_failed_relay_sync(&relay2, event2.clone())
            .await
            .expect("Failed to add retry");

        // Get retries for relay1 only
        let retries = store
            .get_pending_relay_retries(&relay1, 10)
            .await
            .expect("Failed to get retries");

        assert_eq!(retries.len(), 1);
        assert_eq!(retries[0].id, event1.id);
    }

    #[tokio::test]
    async fn test_get_pending_relay_retries_respects_limit() {
        let store = get_store().await;
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        let keys = nostr::key::Keys::generate();

        // Add 5 events
        for i in 0..5 {
            let event = nostr::event::EventBuilder::new(
                nostr::event::Kind::TextNote,
                format!("test {}", i),
            )
            .finalize(&keys)
            .expect("Failed to sign event");
            store
                .add_failed_relay_sync(&relay, event)
                .await
                .expect("Failed to add retry");
        }

        // Request only 3
        let retries = store
            .get_pending_relay_retries(&relay, 3)
            .await
            .expect("Failed to get retries");

        assert_eq!(retries.len(), 3);
    }

    // === Pending Contact Share Tests ===

    fn get_test_pending_share(
        id: &str,
        node_id: &NodeId,
        receiver_node_id: &NodeId,
        direction: ShareDirection,
    ) -> PendingContactShare {
        let keys = BcrKeys::new();
        PendingContactShare {
            id: id.to_string(),
            node_id: node_id.clone(),
            contact: bcr_ebill_core::application::contact::Contact {
                t: bcr_ebill_core::protocol::blockchain::bill::block::ContactType::Person,
                node_id: node_id.clone(),
                name: Name::new("Test Contact").unwrap(),
                email: None,
                postal_address: None,
                date_of_birth_or_registration: None,
                country_of_birth_or_registration: None,
                city_of_birth_or_registration: None,
                identification_number: None,
                avatar_file: None,
                proof_document_file: None,
                nostr_relays: vec![],
                is_logical: false,
                mint_url: None,
            },
            sender_node_id: node_id.clone(),
            contact_private_key: keys.get_private_key(),
            receiver_node_id: receiver_node_id.clone(),
            received_at: Timestamp::now(),
            direction,
            initial_share_id: Some("initial-123".to_string()),
        }
    }

    #[tokio::test]
    async fn test_pending_share_exists_distinguishes_direction() {
        let store = get_store().await;

        let alice_keys = BcrKeys::new();
        let bob_keys = BcrKeys::new();
        let alice_node_id = NodeId::new(alice_keys.pub_key(), bitcoin::Network::Testnet);
        let bob_node_id = NodeId::new(bob_keys.pub_key(), bitcoin::Network::Testnet);

        // Scenario: Alice shares with Bob (Incoming for Bob from Alice)
        let alice_to_bob_share = get_test_pending_share(
            "alice-to-bob",
            &alice_node_id,
            &bob_node_id,
            ShareDirection::Incoming,
        );

        store
            .add_pending_share(alice_to_bob_share)
            .await
            .expect("Failed to add Alice->Bob share");

        // Check: Incoming share from Alice to Bob should exist
        let incoming_exists = store
            .pending_share_exists_for_node_and_receiver(
                &alice_node_id,
                &bob_node_id,
                ShareDirection::Incoming,
            )
            .await
            .expect("Failed to check incoming share");
        assert!(incoming_exists, "Incoming share Alice->Bob should exist");

        // Check: Outgoing share from Alice to Bob should NOT exist
        let outgoing_exists = store
            .pending_share_exists_for_node_and_receiver(
                &alice_node_id,
                &bob_node_id,
                ShareDirection::Outgoing,
            )
            .await
            .expect("Failed to check outgoing share");
        assert!(
            !outgoing_exists,
            "Outgoing share Alice->Bob should NOT exist (only Incoming exists)"
        );

        // Now: Bob shares back with Alice (Incoming for Alice from Bob)
        let bob_to_alice_share = get_test_pending_share(
            "bob-to-alice",
            &bob_node_id,
            &alice_node_id,
            ShareDirection::Incoming,
        );

        store
            .add_pending_share(bob_to_alice_share)
            .await
            .expect("Failed to add Bob->Alice share");

        // Check: Incoming share from Bob to Alice should exist
        let incoming_exists_reverse = store
            .pending_share_exists_for_node_and_receiver(
                &bob_node_id,
                &alice_node_id,
                ShareDirection::Incoming,
            )
            .await
            .expect("Failed to check incoming share reverse");
        assert!(
            incoming_exists_reverse,
            "Incoming share Bob->Alice should exist (bidirectional works!)"
        );

        // Check: Both shares coexist
        let alice_to_bob_still_exists = store
            .pending_share_exists_for_node_and_receiver(
                &alice_node_id,
                &bob_node_id,
                ShareDirection::Incoming,
            )
            .await
            .expect("Failed to check Alice->Bob still exists");
        assert!(
            alice_to_bob_still_exists,
            "Alice->Bob share should still exist after Bob->Alice added"
        );
    }
}
