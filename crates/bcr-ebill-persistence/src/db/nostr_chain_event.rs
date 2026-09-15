use super::{
    Error, Result,
    surreal::{Bindings, SurrealWrapper},
};
use crate::{
    constants::DB_TABLE,
    nostr::{NostrChainEvent, NostrChainEventStoreApi},
};
use async_trait::async_trait;
use bcr_ebill_core::{
    application::ServiceTraitBounds, protocol::Sha256Hash, protocol::Timestamp,
    protocol::blockchain::BlockchainType,
};
use nostr::event::Event;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct SurrealNostrChainEventStore {
    #[allow(dead_code)]
    db: SurrealWrapper,
}
//
// columns
const CHAIN_ID: &str = "chain_id";
const CHAIN_TYPE: &str = "chain_type";
const BLOCK_HASH: &str = "block_hash";
const BLOCK_HEIGHT: &str = "block_height";
const ROOT_ID: &str = "root_id";
const EVENT_ID: &str = "event_id";

impl SurrealNostrChainEventStore {
    const TABLE: &'static str = "nostr_chain_event";

    #[allow(dead_code)]
    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }

    async fn find_all_chain_events(
        &self,
        chain_id: String,
        chain_type: BlockchainType,
    ) -> Result<Vec<NostrChainEventDb>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(CHAIN_ID, chain_id.to_owned())?;
        bindings.add(CHAIN_TYPE, chain_type)?;

        let result: Vec<NostrChainEventDb> = self.db
                .query(format!(
                    "SELECT * FROM type::table(${DB_TABLE}) WHERE {CHAIN_ID} = ${CHAIN_ID} AND {CHAIN_TYPE} = ${CHAIN_TYPE} ORDER BY {BLOCK_HEIGHT} DESC"
                ).as_str(), bindings)
                .await?;
        Ok(result)
    }
}

impl ServiceTraitBounds for SurrealNostrChainEventStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NostrChainEventStoreApi for SurrealNostrChainEventStore {
    async fn find_chain_events(
        &self,
        chain_id: &str,
        chain_type: BlockchainType,
    ) -> Result<Vec<NostrChainEvent>> {
        let result: Vec<NostrChainEventDb> = self
            .find_all_chain_events(chain_id.to_owned(), chain_type)
            .await?;
        Ok(result
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_>>()?)
    }

    async fn find_latest_block_events(
        &self,
        chain_id: &str,
        chain_type: BlockchainType,
    ) -> Result<Vec<NostrChainEvent>> {
        let result: Vec<NostrChainEventDb> = self
            .find_all_chain_events(chain_id.to_owned(), chain_type)
            .await?;
        // Find the highest block_height
        let max_height = result.first().map(|e| e.block_height);
        let latest: Vec<NostrChainEventDb> = match max_height {
            Some(height) => result
                .into_iter()
                .filter(|e| e.block_height == height)
                .collect(),
            None => vec![],
        };
        Ok(latest
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_>>()?)
    }

    async fn find_by_block_hash(&self, hash: &Sha256Hash) -> Result<Option<NostrChainEvent>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(BLOCK_HASH, hash.to_owned())?;

        let result: Vec<NostrChainEventDb> = self
            .db
            .query(
                format!(
                    "SELECT * FROM type::table(${DB_TABLE}) WHERE {BLOCK_HASH} = ${BLOCK_HASH} ORDER BY {BLOCK_HEIGHT} DESC, received DESC"
                )
                .as_str(),
                bindings,
            )
            .await?;

        let value = result
            .first()
            .map(|v| v.to_owned().try_into())
            .transpose()?;
        Ok(value)
    }

    async fn add_chain_event(&self, event: NostrChainEvent) -> Result<()> {
        let db_data: NostrChainEventDb = event.clone().try_into()?;
        let _: Option<NostrChainEventDb> = self
            .db
            .upsert(Self::TABLE, event.event_id.to_owned(), db_data)
            .await?;
        Ok(())
    }

    async fn by_event_id(&self, event_id: &str) -> Result<Option<NostrChainEvent>> {
        let event_id = event_id.to_owned();
        let result: Option<NostrChainEventDb> = self.db.select_one(Self::TABLE, event_id).await?;
        Ok(result.map(|r| r.try_into()).transpose()?)
    }

    async fn find_root_event(
        &self,
        chain_id: &str,
        chain_type: BlockchainType,
    ) -> Result<Option<NostrChainEvent>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(CHAIN_ID, chain_id.to_owned())?;
        bindings.add(CHAIN_TYPE, chain_type)?;

        let result: Vec<NostrChainEventDb> = self.db
                .query(format!(
                    "SELECT * FROM type::table(${DB_TABLE}) WHERE {CHAIN_ID} = ${CHAIN_ID} AND {CHAIN_TYPE} = ${CHAIN_TYPE} AND {EVENT_ID} = {ROOT_ID}"
                ).as_str(), bindings)
                .await?;
        Ok(result
            .first()
            .map(|r| r.to_owned().try_into())
            .transpose()?)
    }

    async fn remove_chain_events(&self, chain_id: &str, chain_type: BlockchainType) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(CHAIN_ID, chain_id.to_owned())?;
        bindings.add(CHAIN_TYPE, chain_type)?;

        self.db
            .query_check(
                format!(
                    "DELETE FROM type::table(${DB_TABLE}) WHERE {CHAIN_ID} = ${CHAIN_ID} AND {CHAIN_TYPE} = ${CHAIN_TYPE}"
                )
                .as_str(),
                bindings,
            )
            .await?;
        Ok(())
    }
}

/// Nostr event for the DB
/// Note: upgrading from
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrEventDb {
    id: nostr::event::EventId,
    pubkey: nostr::key::PublicKey,
    created_at: nostr::types::Timestamp,
    kind: nostr::event::Kind,
    tags: nostr::event::Tags,
    content: String,
    // Use the pre-nostr-0.45 representation as bytes for backwards compat
    sig: bitcoin::secp256k1::schnorr::Signature,
}

impl TryFrom<Event> for NostrEventDb {
    type Error = Error;
    fn try_from(event: Event) -> Result<Self> {
        Ok(Self {
            id: event.id,
            pubkey: event.pubkey,
            created_at: event.created_at,
            kind: event.kind,
            tags: event.tags,
            content: event.content,
            sig: bitcoin::secp256k1::schnorr::Signature::from_slice(event.sig.as_bytes()).map_err(
                |e| {
                    Error::Persistence(format!(
                        "Could not create schnorr Signature from nostr signature: {e}"
                    ))
                },
            )?,
        })
    }
}

impl TryFrom<NostrEventDb> for Event {
    type Error = Error;
    fn try_from(value: NostrEventDb) -> Result<Self> {
        let sig = nostr::event::Signature::from_slice(&value.sig.serialize()).map_err(|e| {
            Error::Persistence(format!(
                "Could not create nostr Signature from persisted signature: {e}"
            ))
        })?;

        Ok(Event::new(
            value.id,
            value.pubkey,
            value.created_at,
            value.kind,
            value.tags,
            value.content,
            sig,
        ))
    }
}

/// Data we need to communicate with a Nostr contact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrChainEventDb {
    /// The nostr event id of this event.
    pub event_id: String,
    /// The event id that started the thread.
    pub root_id: String,
    /// The event id this event was appended to.
    pub reply_id: Option<String>,
    /// The npub of the sender of this event.
    pub author: String,
    /// The BCR id of the blockchain.
    pub chain_id: String,
    /// The type of the blockchain.
    pub chain_type: BlockchainType,
    /// The block height of the block contained in this event.
    pub block_height: usize,
    /// The hash of the block contained in this event.
    pub block_hash: Sha256Hash,
    /// The timestamp when we received the event.
    pub received: Timestamp,
    /// The timestamp of the event.
    pub time: Timestamp,
    /// The event as we received it via nostr.
    pub payload: NostrEventDb,
}

impl TryFrom<NostrChainEvent> for NostrChainEventDb {
    type Error = Error;
    fn try_from(event: NostrChainEvent) -> Result<Self> {
        Ok(Self {
            event_id: event.event_id,
            root_id: event.root_id,
            reply_id: event.reply_id,
            author: event.author,
            chain_id: event.chain_id,
            chain_type: event.chain_type,
            block_height: event.block_height,
            block_hash: event.block_hash,
            received: event.received,
            time: event.time,
            payload: event.payload.try_into()?,
        })
    }
}

impl TryFrom<NostrChainEventDb> for NostrChainEvent {
    type Error = Error;

    fn try_from(db: NostrChainEventDb) -> Result<Self> {
        Ok(Self {
            event_id: db.event_id,
            root_id: db.root_id,
            reply_id: db.reply_id,
            author: db.author,
            chain_id: db.chain_id,
            chain_type: db.chain_type,
            block_height: db.block_height,
            block_hash: db.block_hash,
            received: db.received,
            time: db.time,
            payload: Event::try_from(db.payload)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use bcr_ebill_core::{protocol::Timestamp, protocol::crypto::BcrKeys};
    use nostr::event::{EventBuilder, FinalizeEvent};

    use super::*;
    use crate::db::get_memory_db;

    #[tokio::test]
    async fn test_add_event() {
        let store = get_store("add_event").await;
        let event = get_root_event();
        let res = store.add_chain_event(event.clone()).await;
        assert!(res.is_ok());

        let stored = store
            .by_event_id(&event.event_id)
            .await
            .expect("could not query event by id");
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn test_event_by_hash() {
        let store = get_store("event_by_hash").await;
        let root = get_root_event();
        let child = get_child_event("child_id", 2, &Sha256Hash::new("child_hash"), &root, None);
        store
            .add_chain_event(root)
            .await
            .expect("root event creation failed");
        store
            .add_chain_event(child)
            .await
            .expect("child event creation failed");
        let by_hash = store
            .find_by_block_hash(&Sha256Hash::new("child_hash"))
            .await
            .expect("could not find by hash");
        assert!(
            by_hash.is_some(),
            "Expected item by hash to return something"
        );
        assert_eq!(by_hash.unwrap().event_id, "child_id");
    }

    #[tokio::test]
    async fn test_find_root_event() {
        let store = get_store("find_root_event").await;
        let root = get_root_event();
        let child = get_child_event("child_id", 2, &Sha256Hash::new("child_hash"), &root, None);
        store
            .add_chain_event(root)
            .await
            .expect("root event creation failed");
        store
            .add_chain_event(child)
            .await
            .expect("child event creation failed");

        let root_result = store
            .find_root_event("chain_id", BlockchainType::Bill)
            .await
            .expect("could not find root event");

        assert!(
            root_result.is_some(),
            "Expected find root event to return something"
        );
        assert_eq!(root_result.unwrap().event_id, "root_event_id");
    }

    #[tokio::test]
    async fn test_find_latest_block_events() {
        let store = get_store("find_latest_block_events").await;
        let root = get_root_event();
        let child = get_child_event(
            "child_event",
            2,
            &Sha256Hash::new("child_hash"),
            &root,
            None,
        );
        let target1 = get_child_event(
            "child_event_a",
            3,
            &Sha256Hash::new("child_hash_a"),
            &root,
            Some(&child),
        );
        let target2 = get_child_event(
            "child_event_b",
            3,
            &Sha256Hash::new("child_hash_b"),
            &root,
            Some(&child),
        );
        store
            .add_chain_event(root)
            .await
            .expect("root event creation failed");
        store
            .add_chain_event(child)
            .await
            .expect("child event creation failed");
        store
            .add_chain_event(target1)
            .await
            .expect("target event creation failed");

        let latest = store
            .find_latest_block_events("chain_id", BlockchainType::Bill)
            .await
            .expect("could not find latest block events");
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].event_id, "child_event_a");

        store
            .add_chain_event(target2)
            .await
            .expect("target event creation failed");

        let latest = store
            .find_latest_block_events("chain_id", BlockchainType::Bill)
            .await
            .expect("could not find latest block events");
        assert_eq!(latest.len(), 2);
    }

    #[tokio::test]
    async fn test_find_all_events() {
        let store = get_store("find_all_events").await;
        let root = get_root_event();
        let child = get_child_event(
            "child_event",
            2,
            &Sha256Hash::new("child_hash"),
            &root,
            None,
        );
        let target1 = get_child_event(
            "child_event_a",
            3,
            &Sha256Hash::new("child_hash_a"),
            &root,
            Some(&child),
        );
        let target2 = get_child_event(
            "child_event_b",
            3,
            &Sha256Hash::new("child_hash_b"),
            &root,
            Some(&child),
        );

        store
            .add_chain_event(root)
            .await
            .expect("root event creation failed");
        store
            .add_chain_event(child)
            .await
            .expect("child event creation failed");
        store
            .add_chain_event(target1)
            .await
            .expect("target event creation failed");
        store
            .add_chain_event(target2)
            .await
            .expect("target event creation failed");

        let all = store
            .find_chain_events("chain_id", BlockchainType::Bill)
            .await
            .expect("could not find all events");

        assert_eq!(all.len(), 4);
    }
    async fn get_store(db: &str) -> SurrealNostrChainEventStore {
        let mem_db = get_memory_db("test", db)
            .await
            .expect("could not create memory db");
        SurrealNostrChainEventStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    fn get_root_event() -> NostrChainEvent {
        get_test_chain_event(
            "root_event_id",
            "root_event_id",
            None,
            1,
            &Sha256Hash::new("root_hash"),
        )
    }

    fn get_child_event(
        id: &str,
        height: usize,
        hash: &Sha256Hash,
        root: &NostrChainEvent,
        parent: Option<&NostrChainEvent>,
    ) -> NostrChainEvent {
        get_test_chain_event(
            id,
            &root.event_id,
            parent.map(|p| p.event_id.to_owned()),
            height,
            hash,
        )
    }

    fn get_test_chain_event(
        event_id: &str,
        root_id: &str,
        reply_id: Option<String>,
        block_height: usize,
        block_hash: &Sha256Hash,
    ) -> NostrChainEvent {
        NostrChainEvent {
            event_id: event_id.to_string(),
            root_id: root_id.to_string(),
            reply_id,
            author: "author".to_string(),
            chain_id: "chain_id".to_string(),
            chain_type: BlockchainType::Bill,
            block_height,
            block_hash: block_hash.to_owned(),
            received: Timestamp::now(),
            time: Timestamp::now(),
            payload: get_test_event(),
        }
    }

    fn get_test_event() -> Event {
        let keys = BcrKeys::new().get_nostr_keys();
        EventBuilder::new(nostr::event::Kind::TextNote, "content")
            .finalize(&keys)
            .expect("could not create nostr test event")
    }

    // 0.45 upgrade regression test
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct LegacyNostrChainEventDb {
        event_id: String,
        root_id: String,
        reply_id: Option<String>,
        author: String,
        chain_id: String,
        chain_type: BlockchainType,
        block_height: usize,
        block_hash: Sha256Hash,
        received: Timestamp,
        time: Timestamp,

        // Actual historical Event implementation
        payload: nostr_043::event::Event,
    }

    fn get_legacy_event() -> nostr_043::event::Event {
        let keys = nostr_043::key::Keys::generate();

        nostr_043::event::EventBuilder::new(nostr_043::event::Kind::TextNote, "legacy content")
            .sign_with_keys(&keys)
            .expect("could not create legacy event")
    }

    #[tokio::test]
    async fn test_reads_chain_event_persisted_with_nostr_0_43() {
        let store = get_store("nostr_043_compat").await;

        let legacy_event = get_legacy_event();

        let event_id = "legacy-event-id";

        let legacy = LegacyNostrChainEventDb {
            event_id: event_id.to_string(),
            root_id: event_id.to_string(),
            reply_id: None,
            author: "author".to_string(),
            chain_id: "chain_id".to_string(),
            chain_type: BlockchainType::Bill,
            block_height: 1,
            block_hash: Sha256Hash::new("root_hash"),
            received: Timestamp::now(),
            time: Timestamp::now(),
            payload: legacy_event,
        };

        // write using 0.43
        let _: Option<LegacyNostrChainEventDb> = store
            .db
            .upsert(
                SurrealNostrChainEventStore::TABLE,
                event_id.to_string(),
                legacy,
            )
            .await
            .expect("could not persist nostr 0.43 event");

        // read using 0.45+
        let loaded = store
            .by_event_id(event_id)
            .await
            .expect("could not read nostr 0.43 event")
            .expect("event not found");

        assert_eq!(loaded.chain_id, "chain_id");
        assert_eq!(loaded.payload.content, "legacy content");
    }
}
