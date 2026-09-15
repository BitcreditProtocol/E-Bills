use super::{
    Result,
    surreal::{Bindings, SurrealWrapper},
};
use crate::constants::{DB_IDS, DB_LIMIT, DB_TABLE, NOSTR_QUEUE_PROCESSING_TIMEOUT_SECS};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::DateTimeUtc, protocol::Timestamp};
use serde::{Deserialize, Serialize};
use surrealdb::sql::Thing;
use time::{UtcOffset, format_description::well_known::Rfc3339};

use crate::nostr::{NostrQueuedMessage, NostrQueuedMessageStoreApi};

#[derive(Clone)]
pub struct SurrealNostrEventQueueStore {
    db: SurrealWrapper,
}

impl SurrealNostrEventQueueStore {
    const TABLE: &'static str = "nostr_event_send_queue";

    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }

    async fn set_processing_started_at(
        &self,
        ids: Vec<Thing>,
        started_at: DateTimeUtc,
    ) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_IDS, ids)?;
        bindings.add("started_at", datetime_db_value(started_at))?;
        self.db
            .query_check(
                "UPDATE type::table($table) SET processing_started_at = $started_at WHERE id IN $ids",
                bindings,
            )
            .await?;
        Ok(())
    }
}

impl ServiceTraitBounds for SurrealNostrEventQueueStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NostrQueuedMessageStoreApi for SurrealNostrEventQueueStore {
    /// Adds a new retry message
    async fn add_message(&self, message: NostrQueuedMessage, max_retries: i32) -> Result<()> {
        let id = message.id.to_owned();
        let message = QueuedMessageDb::from(message, max_retries);
        let _: Option<QueuedMessageDb> = self
            .db
            .create(Self::TABLE, Some(id.to_owned()), message)
            .await?;
        Ok(())
    }
    /// Selects all messages that are ready to be retried
    async fn get_retry_messages(&self, limit: u64) -> Result<Vec<NostrQueuedMessage>> {
        let now = Timestamp::now();
        let retry_before = now
            .inner()
            .saturating_sub(NOSTR_QUEUE_PROCESSING_TIMEOUT_SECS);
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_LIMIT, limit)?;
        bindings.add(
            "retry_before",
            datetime_db_value(Timestamp::new(retry_before).expect("safe").to_datetime()),
        )?;
        let items: Vec<QueuedMessageDb> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE completed = false AND processing_started_at < $retry_before ORDER BY last_try ASC LIMIT $limit", bindings)
            .await?;
        let ids = items.iter().map(|i| i.id.to_owned()).collect();
        let results: Vec<NostrQueuedMessage> = items.into_iter().map(|i| i.into()).collect();
        self.set_processing_started_at(ids, now.to_datetime())
            .await?;
        Ok(results)
    }

    /// Fail a retry attempt, schedules a new retry or fails the message if
    /// all retries have been exhausted.
    async fn fail_retry(&self, id: &str) -> Result<()> {
        let current: Option<QueuedMessageDb> =
            self.db.select_one(Self::TABLE, id.to_owned()).await?;
        if let Some(mut msg) = current {
            msg.num_retries += 1;
            msg.last_try = Timestamp::now().to_datetime();
            msg.completed = msg.num_retries >= msg.max_retries;
            msg.processing_started_at = Timestamp::zero().to_datetime();
            let _: Option<QueuedMessageDb> =
                self.db.update(Self::TABLE, id.to_owned(), msg).await?;
        }
        Ok(())
    }
    /// Flags a retry as successful
    async fn succeed_retry(&self, id: &str) -> Result<()> {
        let current: Option<QueuedMessageDb> =
            self.db.select_one(Self::TABLE, id.to_owned()).await?;
        if let Some(mut msg) = current {
            msg.completed = true;
            msg.last_try = Timestamp::now().to_datetime();
            msg.processing_started_at = Timestamp::zero().to_datetime();
            let _: Option<QueuedMessageDb> =
                self.db.update(Self::TABLE, id.to_owned(), msg).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedMessageDb {
    pub id: Thing,
    pub sender_id: NodeId,
    #[serde(alias = "node_id")]
    pub recipient: Option<NodeId>,
    pub payload: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created: DateTimeUtc,
    #[serde(with = "time::serde::rfc3339")]
    pub last_try: DateTimeUtc,
    pub num_retries: i32,
    pub max_retries: i32,
    pub completed: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub processing_started_at: DateTimeUtc,
}

impl QueuedMessageDb {
    fn from(value: NostrQueuedMessage, max_retries: i32) -> Self {
        QueuedMessageDb {
            id: Thing::from((
                SurrealNostrEventQueueStore::TABLE.to_owned(),
                value.id.to_owned(),
            )),
            sender_id: value.sender_id,
            recipient: value.recipient,
            payload: value.payload,
            created: Timestamp::now().to_datetime(),
            last_try: Timestamp::new(0).expect("safe").to_datetime(),
            num_retries: 0,
            max_retries,
            completed: false,
            processing_started_at: Timestamp::zero().to_datetime(),
        }
    }
}

impl From<QueuedMessageDb> for NostrQueuedMessage {
    fn from(value: QueuedMessageDb) -> Self {
        NostrQueuedMessage {
            id: value.id.id.to_raw(),
            sender_id: value.sender_id,
            recipient: value.recipient,
            payload: value.payload,
        }
    }
}

fn datetime_db_value(dt: DateTimeUtc) -> String {
    dt.to_offset(UtcOffset::UTC)
        .format(&Rfc3339)
        .expect("datetime can be formatted as RFC3339")
}

#[cfg(test)]
mod tests {
    use bitcoin::base58;

    use super::*;
    use crate::{db::get_memory_db, tests::tests::node_id_test};

    #[tokio::test]
    async fn test_insert_query_and_mark_succeeded() {
        let store = get_store().await;
        store
            .add_message(get_test_message("test_message"), 3)
            .await
            .expect("could not add message");

        let messages = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");
        assert!(!messages.is_empty(), "should have gotten a queued message");

        let messages_empty = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");

        assert!(
            messages_empty.is_empty(),
            "should not have gotten a queued message"
        );

        store
            .succeed_retry(&messages[0].id)
            .await
            .expect("could not mark message as succeeded");

        let messages_done = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");
        assert!(
            messages_done.is_empty(),
            "should not have gotten a queued message"
        );
    }

    #[tokio::test]
    async fn test_insert_query_and_mark_failed() {
        let store = get_store().await;
        store
            .add_message(get_test_message("test_message"), 2)
            .await
            .expect("could not add message");

        let messages = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");
        assert!(!messages.is_empty(), "should have gotten a queued message");

        let messages_empty = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");

        assert!(
            messages_empty.is_empty(),
            "should not have gotten a queued message"
        );

        store
            .fail_retry(&messages[0].id)
            .await
            .expect("could not mark message as failed");

        let messages_failed = store
            .get_retry_messages(1)
            .await
            .expect("could not get failed messages");

        assert!(
            !messages_failed.is_empty(),
            "should have gotten a failed message"
        );

        store
            .fail_retry(&messages_failed[0].id)
            .await
            .expect("could not mark message as failed");

        let messages_failed_again = store
            .get_retry_messages(1)
            .await
            .expect("could not get failed messages");

        assert!(
            messages_failed_again.is_empty(),
            "should have exceeded retry limit"
        );
    }

    #[tokio::test]
    async fn test_stale_processing_started_at_is_retryable_again() {
        let store = get_store().await;
        store
            .add_message(get_test_message("test_message"), 3)
            .await
            .expect("could not add message");

        let messages = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");
        assert_eq!(messages.len(), 1, "should have gotten a queued message");

        let messages_empty = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");
        assert!(
            messages_empty.is_empty(),
            "lease should block immediate retry"
        );

        let mut bindings = Bindings::default();
        bindings
            .add(DB_TABLE, SurrealNostrEventQueueStore::TABLE)
            .expect("could not bind table");
        bindings
            .add(
                DB_IDS,
                vec![Thing::from((
                    SurrealNostrEventQueueStore::TABLE,
                    messages[0].id.as_str(),
                ))],
            )
            .expect("could not bind ids");
        bindings
            .add(
                "started_at",
                datetime_db_value(Timestamp::zero().to_datetime()),
            )
            .expect("could not bind started_at");
        store
            .db
            .query_check(
                "UPDATE type::table($table) SET processing_started_at = $started_at WHERE id IN $ids",
                bindings,
            )
            .await
            .expect("could not reset stale lease");

        let messages_retried = store
            .get_retry_messages(1)
            .await
            .expect("could not get retried messages");
        assert_eq!(messages_retried.len(), 1, "stale lease should be retried");
    }

    #[tokio::test]
    async fn test_fail_retry_resets_processing_started_at() {
        let store = get_store().await;
        store
            .add_message(get_test_message("test_message"), 3)
            .await
            .expect("could not add message");

        let messages = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");

        store
            .fail_retry(&messages[0].id)
            .await
            .expect("could not mark retry as failed");

        let record: Option<QueuedMessageDb> = store
            .db
            .select_one(SurrealNostrEventQueueStore::TABLE, messages[0].id.clone())
            .await
            .expect("could not load queued message");

        let record = record.expect("queued message should exist");
        assert_eq!(
            record.processing_started_at,
            Timestamp::zero().to_datetime()
        );
    }

    #[tokio::test]
    async fn test_succeed_retry_resets_processing_started_at() {
        let store = get_store().await;
        store
            .add_message(get_test_message("test_message"), 3)
            .await
            .expect("could not add message");

        let messages = store
            .get_retry_messages(1)
            .await
            .expect("could not get messages");

        store
            .succeed_retry(&messages[0].id)
            .await
            .expect("could not mark retry as successful");

        let record: Option<QueuedMessageDb> = store
            .db
            .select_one(SurrealNostrEventQueueStore::TABLE, messages[0].id.clone())
            .await
            .expect("could not load queued message");

        let record = record.expect("queued message should exist");
        assert!(record.completed);
        assert_eq!(
            record.processing_started_at,
            Timestamp::zero().to_datetime()
        );
    }

    async fn get_store() -> SurrealNostrEventQueueStore {
        let mem_db = get_memory_db("test", "nostr_event_queue")
            .await
            .expect("could not create memory db");
        SurrealNostrEventQueueStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    fn get_test_message(id: &str) -> NostrQueuedMessage {
        NostrQueuedMessage {
            id: id.to_string(),
            sender_id: node_id_test(),
            recipient: Some(node_id_test()),
            payload: base58::encode(&borsh::to_vec(r#"{"foo": "bar"}"#).unwrap()),
        }
    }
}
