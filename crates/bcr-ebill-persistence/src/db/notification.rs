use bcr_common::core::{BillId, NodeId};
use std::collections::{HashMap, hash_map::Entry};

use super::{
    super::{Error, Result},
    surreal::{Bindings, SurrealWrapper},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use surrealdb::sql::Thing;

use crate::{
    constants::{DB_ACTIVE, DB_IDS, DB_NOTIFICATION_TYPE, DB_TABLE},
    notification::{NotificationFilter, NotificationStoreApi},
};
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::event::bill_events::ActionType};
use bcr_ebill_core::{
    application::notification::{Notification, NotificationLevel, NotificationType},
    protocol::DateTimeUtc,
    protocol::Timestamp,
};

#[derive(Clone)]
pub struct SurrealNotificationStore {
    db: SurrealWrapper,
}

impl SurrealNotificationStore {
    const TABLE: &'static str = "notifications";
    const SENT_TABLE: &'static str = "sent_notifications";

    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }
}

impl ServiceTraitBounds for SurrealNotificationStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationStoreApi for SurrealNotificationStore {
    /// Returns node ids with an active notification for the given node ids
    async fn get_active_status_for_node_ids(
        &self,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, bool>> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::TABLE)?;
        bindings.add("node_ids", node_ids.to_owned())?;

        let node_id_filter = if node_ids.is_empty() {
            ""
        } else {
            "and node_id in $node_ids"
        };

        let result: Vec<NodeIdDb> = self.db.query(&format!("SELECT node_id from notifications where active = true {node_id_filter} GROUP BY node_id"), bindings).await?;
        let mut res: HashMap<NodeId, bool> = HashMap::new();

        if node_ids.is_empty() {
            for node_id_db in result {
                res.insert(node_id_db.node_id.to_owned(), true);
            }
        } else {
            for node_id in node_ids {
                res.insert(
                    node_id.to_owned(),
                    result.iter().any(|n| n.node_id == *node_id),
                );
            }
        }
        Ok(res)
    }

    /// Stores a new notification into the database
    async fn add(&self, notification: Notification) -> Result<Notification> {
        let id = notification.id.to_owned();
        let entity: NotificationDb = notification.into();
        let result: Option<NotificationDb> = self
            .db
            .create(Self::TABLE, Some(id.to_string()), entity)
            .await?;

        match result {
            Some(n) => Ok(n.into()),
            None => Err(Error::InsertFailed(format!(
                "{} with id {}",
                Self::TABLE,
                id
            ))),
        }
    }
    /// Returns all currently active notifications from the database
    async fn list(&self, filter: NotificationFilter) -> Result<Vec<Notification>> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::TABLE)?;
        bindings.add("limit", filter.get_limit())?;
        bindings.add("offset", filter.get_offset())?;
        let filters = filter.filters();

        if let Some(active) = filter.get_active() {
            bindings.add(&active.0, active.1.to_owned())?;
        }
        if let Some(reference_id) = filter.get_reference_id() {
            bindings.add(&reference_id.0, reference_id.1.to_owned())?;
        }
        if let Some(notification_type) = filter.get_notification_type() {
            bindings.add(&notification_type.0, notification_type.1.to_owned())?;
        }
        if let Some(event_id) = filter.get_event_id() {
            bindings.add(&event_id.0, event_id.1.to_owned())?;
        }
        if let Some(node_ids) = filter.get_node_ids() {
            bindings.add(&node_ids.0, node_ids.1.to_owned())?;
        }
        if let Some(level) = filter.get_level() {
            bindings.add(&level.0, level.1.to_owned())?;
        }
        let result: Vec<NotificationDb> = self.db.query(&format!(
                "SELECT * FROM type::table($table) {filters} ORDER BY datetime DESC LIMIT $limit START $offset"
            ), bindings).await?;
        Ok(result.into_iter().map(|n| n.into()).collect())
    }
    /// Returns the latest active notifications for the given reference and notification type
    async fn get_latest_by_references(
        &self,
        references: &[String],
        notification_type: NotificationType,
    ) -> Result<HashMap<String, Notification>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::TABLE)?;
        bindings.add(DB_ACTIVE, true)?;
        bindings.add(DB_NOTIFICATION_TYPE, notification_type.to_owned())?;
        bindings.add(DB_IDS, references.to_owned())?;
        let result: Vec<NotificationDb> = self
            .db
            .query(
                "SELECT * FROM type::table($table) WHERE active = $active AND notification_type = $notification_type AND reference_id IN $ids",bindings
            )
            .await?;
        let mut latest_map: HashMap<String, Notification> = HashMap::new();

        for notification in result {
            if let Some(ref_id) = &notification.reference_id {
                let entry = latest_map.entry(ref_id.clone());

                match entry {
                    Entry::Vacant(e) => {
                        e.insert(notification.into());
                    }
                    Entry::Occupied(mut e) => {
                        // only keep the latest one
                        if notification.datetime > e.get().datetime {
                            e.insert(notification.into());
                        }
                    }
                }
            }
        }

        Ok(latest_map)
    }
    /// Returns the latest active notification for the given reference and notification type
    async fn get_latest_by_reference(
        &self,
        reference: &str,
        notification_type: NotificationType,
    ) -> Result<Option<Notification>> {
        let result = self
            .list(NotificationFilter {
                active: Some(true),
                reference_id: Some(reference.to_owned()),
                notification_type: Some(notification_type.to_string()),
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        Ok(result.first().cloned())
    }
    /// Returns the latest active notification for the given reference, notification type and node id
    async fn get_latest_by_reference_and_node_id(
        &self,
        reference: &str,
        notification_type: NotificationType,
        node_id: &NodeId,
    ) -> Result<Option<Notification>> {
        let result = self
            .list(NotificationFilter {
                active: Some(true),
                reference_id: Some(reference.to_owned()),
                notification_type: Some(notification_type.to_string()),
                node_ids: vec![node_id.to_owned()],
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        Ok(result.first().cloned())
    }
    /// Returns all notifications for the given reference and notification type that are active
    async fn list_by_type(&self, notification_type: NotificationType) -> Result<Vec<Notification>> {
        let result = self
            .list(NotificationFilter {
                active: Some(true),
                notification_type: Some(notification_type.to_string()),
                ..Default::default()
            })
            .await?;
        Ok(result)
    }
    /// Marks an active notification as done
    async fn mark_as_done(&self, notification_id: &str) -> Result<()> {
        let thing: Thing = (Self::TABLE, notification_id).into();
        let mut bindings = Bindings::default();
        bindings.add("id", thing)?;
        self.db
            .query_check("UPDATE $id SET active = false", bindings)
            .await?;
        Ok(())
    }
    /// deletes a notification from the database
    async fn delete(&self, notification_id: &str) -> Result<()> {
        let _: Option<NotificationDb> = self
            .db
            .delete(Self::TABLE, notification_id.to_owned())
            .await?;
        Ok(())
    }

    async fn set_bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action_type: ActionType,
    ) -> Result<()> {
        let db = SentBlockNotificationDb {
            notification_type: NotificationType::Bill,
            reference_id: bill_id.to_string(),
            block_height,
            action_type,
            datetime: Timestamp::now().to_datetime(),
        };
        let _: Option<SentBlockNotificationDb> = self.db.create(Self::SENT_TABLE, None, db).await?;
        Ok(())
    }

    async fn bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action_type: ActionType,
    ) -> Result<bool> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::SENT_TABLE)?;
        bindings.add("notification_type", NotificationType::Bill)?;
        bindings.add("reference_id", bill_id.to_string())?;
        bindings.add("block_height", block_height)?;
        bindings.add("action_type", action_type)?;
        let res: Vec<SentBlockNotificationDb> = self.db
            .query("SELECT * FROM type::table($table) WHERE notification_type = $notification_type AND reference_id = $reference_id AND block_height = $block_height AND action_type = $action_type limit 1", bindings)
            .await?;
        Ok(!res.is_empty())
    }

    async fn notification_exists_for_event_id(
        &self,
        event_id: &str,
        node_id: &NodeId,
    ) -> Result<bool> {
        let mut bindings = Bindings::default();
        bindings.add("table", Self::TABLE)?;
        bindings.add("event_id", event_id.to_string())?;
        bindings.add("node_id", node_id.to_owned())?;
        let res: Vec<NotificationDb> = self.db
            .query("SELECT * FROM type::table($table) WHERE event_id = $event_id AND node_id = $node_id limit 1", bindings)
            .await?;
        Ok(!res.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeIdDb {
    pub node_id: NodeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationDb {
    pub id: Thing,
    pub node_id: Option<NodeId>,
    pub notification_type: NotificationType,
    pub reference_id: Option<String>,
    pub description: String,
    #[serde(with = "time::serde::rfc3339")]
    pub datetime: DateTimeUtc,
    pub active: bool,
    #[serde(default)]
    pub level: NotificationLevel,
    pub payload: Option<Value>,
    pub event_id: Option<String>,
}

impl From<NotificationDb> for Notification {
    fn from(value: NotificationDb) -> Self {
        Self {
            id: value.id.id.to_raw(),
            node_id: value.node_id,
            notification_type: value.notification_type,
            reference_id: value.reference_id,
            description: value.description,
            datetime: value.datetime,
            active: value.active,
            level: value.level,
            payload: value.payload,
            event_id: value.event_id,
        }
    }
}

impl From<Notification> for NotificationDb {
    fn from(value: Notification) -> Self {
        Self {
            id: (
                SurrealNotificationStore::TABLE.to_owned(),
                value.id.to_owned(),
            )
                .into(),
            node_id: value.node_id,
            notification_type: value.notification_type,
            reference_id: value.reference_id,
            description: value.description,
            datetime: value.datetime,
            active: value.active,
            level: value.level,
            payload: value.payload,
            event_id: value.event_id,
        }
    }
}

/// Tracks sending of notifications for a blockchain based resource.
/// The block height is used to track the block in which the notification was sent.
/// The same notification can be sent multiple times, but only if the underlying
/// resource has added new blocks to the chain.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct SentBlockNotificationDb {
    pub notification_type: NotificationType,
    pub reference_id: String,
    pub block_height: i32,
    pub action_type: ActionType,
    #[serde(with = "time::serde::rfc3339")]
    pub datetime: DateTimeUtc,
}

#[cfg(test)]
mod tests {
    use bcr_ebill_core::protocol::Timestamp;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::{
        db::get_memory_db,
        tests::tests::{bill_id_test, bill_id_test_other, node_id_test, node_id_test_other},
    };

    async fn get_store() -> SurrealNotificationStore {
        let db = get_memory_db("test", "notification")
            .await
            .expect("could not create memory db");
        SurrealNotificationStore::new(SurrealWrapper { db, files: false })
    }

    #[tokio::test]
    async fn test_notification_sent_returns_false_for_non_existing() {
        let store = get_store().await;
        let sent = store
            .bill_notification_sent(&bill_id_test(), 1, ActionType::AcceptBill)
            .await
            .expect("could not check if notification was sent");
        assert!(!sent);
    }

    #[tokio::test]
    async fn test_notification_sent_returns_true_for_existing() {
        let store = get_store().await;
        store
            .set_bill_notification_sent(&bill_id_test(), 1, ActionType::AcceptBill)
            .await
            .expect("could not set notification as sent");
        let sent = store
            .bill_notification_sent(&bill_id_test(), 1, ActionType::AcceptBill)
            .await
            .expect("could not check if notification was sent");
        assert!(sent);
    }

    #[tokio::test]
    async fn test_notification_sent_returns_false_for_different_notification_type() {
        let store = get_store().await;
        store
            .set_bill_notification_sent(&bill_id_test(), 1, ActionType::AcceptBill)
            .await
            .expect("could not set notification as sent");
        let sent = store
            .bill_notification_sent(&bill_id_test(), 1, ActionType::PayBill)
            .await
            .expect("could not check if notification was sent");
        assert!(!sent);
    }

    #[tokio::test]
    async fn test_inserts_and_queries_notification() {
        let store = get_store().await;
        let notification = test_notification(&bill_id_test(), Some(test_payload()));
        let r = store
            .add(notification.clone())
            .await
            .expect("could not create notification");

        let all = store
            .list(NotificationFilter::default())
            .await
            .expect("could not list notifications");
        assert!(!all.is_empty());
        assert_eq!(notification.id, r.id);
    }

    #[tokio::test]
    async fn test_deletes_existing_notification() {
        let store = get_store().await;
        let notification = test_notification(&bill_id_test(), Some(test_payload()));
        let r = store
            .add(notification.clone())
            .await
            .expect("could not create notification");

        let all = store
            .list(NotificationFilter::default())
            .await
            .expect("could not list notifications");
        assert!(!all.is_empty());

        store
            .delete(&r.id)
            .await
            .expect("could not delete notification");
        let all = store
            .list(NotificationFilter::default())
            .await
            .expect("could not list notifications");
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_marks_done_and_no_longer_returns_in_list() {
        let store = get_store().await;
        let notification = test_notification(&bill_id_test(), Some(test_payload()));
        let r = store
            .add(notification.clone())
            .await
            .expect("could not create notification");

        let filter = NotificationFilter {
            active: Some(true),
            ..Default::default()
        };
        let all = store
            .list(filter.clone())
            .await
            .expect("could not list notifications");
        assert!(!all.is_empty());

        store
            .mark_as_done(&r.id)
            .await
            .expect("could not mark notification as done");

        let all = store
            .list(filter)
            .await
            .expect("could not list notifications");
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_marks_done_and_no_longer_returns_by_references() {
        let store = get_store().await;
        let notification = test_notification(&bill_id_test(), Some(test_payload()));
        let notification2 = test_notification(&bill_id_test_other(), Some(test_payload()));
        let r = store
            .add(notification.clone())
            .await
            .expect("could not create notification");
        store
            .add(notification2.clone())
            .await
            .expect("could not create notification");

        let references = store
            .get_latest_by_references(
                &[bill_id_test().to_string(), bill_id_test_other().to_string()],
                NotificationType::Bill,
            )
            .await
            .expect("could not list notifications");
        assert_eq!(references.len(), 2);
        assert_eq!(
            references
                .get(&bill_id_test().to_string())
                .unwrap()
                .reference_id
                .as_ref()
                .unwrap(),
            &bill_id_test().to_string()
        );
        assert_eq!(
            references
                .get(&bill_id_test_other().to_string())
                .unwrap()
                .reference_id
                .as_ref()
                .unwrap(),
            &bill_id_test_other().to_string()
        );

        store
            .mark_as_done(&r.id)
            .await
            .expect("could not mark notification as done");

        let references = store
            .get_latest_by_references(
                &[bill_id_test().to_string(), bill_id_test_other().to_string()],
                NotificationType::Bill,
            )
            .await
            .expect("could not list notifications");
        assert_eq!(references.len(), 1);
        assert_eq!(
            references
                .get(&bill_id_test_other().to_string())
                .unwrap()
                .reference_id
                .as_ref()
                .unwrap(),
            &bill_id_test_other().to_string()
        );
    }

    #[tokio::test]
    async fn test_marks_done_and_no_longer_returns_by_reference() {
        let store = get_store().await;
        let notification = test_notification(&bill_id_test(), Some(test_payload()));
        let r = store
            .add(notification.clone())
            .await
            .expect("could not create notification");

        let latest = store
            .get_latest_by_reference(
                &notification.clone().reference_id.unwrap(),
                NotificationType::Bill,
            )
            .await
            .expect("could not list notifications");
        assert!(latest.is_some());

        store
            .mark_as_done(&r.id)
            .await
            .expect("could not mark notification as done");

        let latest = store
            .get_latest_by_reference(
                &notification.clone().reference_id.unwrap(),
                NotificationType::Bill,
            )
            .await
            .expect("could not list notifications");

        assert!(latest.is_none());
    }

    #[tokio::test]
    async fn test_returns_all_active_by_type() {
        let store = get_store().await;
        let notification1 = test_notification(&bill_id_test(), Some(test_payload()));
        let notification2 = test_notification(&bill_id_test_other(), Some(test_payload()));
        let notification3 = test_general_notification();
        let _ = store
            .add(notification1.clone())
            .await
            .expect("notification created");
        let _ = store
            .add(notification2.clone())
            .await
            .expect("notification created");
        let _ = store
            .add(notification3.clone())
            .await
            .expect("notification created");
        store
            .mark_as_done(&notification2.clone().id)
            .await
            .expect("notification marked done");
        let by_type = store
            .list_by_type(NotificationType::Bill)
            .await
            .expect("returned list by type");

        assert_eq!(by_type.len(), 1, "should only have one bill type in list");
        by_type.iter().for_each(|n| {
            assert!(n.active);
            assert_ne!(
                n.id, notification2.id,
                "notfication 2 should be done already"
            );
        });
    }

    #[tokio::test]
    async fn test_returns_active_status_for_node_ids() {
        let store = get_store().await;
        let notification1 = test_notification(&bill_id_test(), Some(test_payload()));
        let mut notification2 = test_notification(&bill_id_test_other(), Some(test_payload()));
        notification2.node_id = Some(node_id_test_other());
        let notification3 = test_general_notification();
        let _ = store
            .add(notification1.clone())
            .await
            .expect("notification created");
        let _ = store
            .add(notification2.clone())
            .await
            .expect("notification created");
        let _ = store
            .add(notification3.clone())
            .await
            .expect("notification created");

        let status = store
            .get_active_status_for_node_ids(&[])
            .await
            .expect("returns status");

        assert_eq!(status.len(), 2, "should have all node ids in list");
        assert!(status.get(&node_id_test()).unwrap());
        assert!(status.get(&node_id_test_other()).unwrap());

        let status = store
            .get_active_status_for_node_ids(&[node_id_test()])
            .await
            .expect("returns status");

        assert_eq!(
            status.len(),
            1,
            "should have all given node ids in the list"
        );
        assert!(status.get(&node_id_test()).unwrap());

        store
            .mark_as_done(&notification2.clone().id)
            .await
            .expect("notification marked done");

        let status = store
            .get_active_status_for_node_ids(&[node_id_test_other()])
            .await
            .expect("returns status");

        assert_eq!(
            status.len(),
            1,
            "should have all given node ids in the list"
        );
        assert!(!status.get(&node_id_test_other()).unwrap());

        let status = store
            .get_active_status_for_node_ids(&[node_id_test(), node_id_test_other()])
            .await
            .expect("returns status");

        assert_eq!(status.len(), 2, "should have all given node ids in list");
        assert!(status.get(&node_id_test()).unwrap());
        assert!(!status.get(&node_id_test_other()).unwrap());

        let status = store
            .get_active_status_for_node_ids(&[])
            .await
            .expect("returns status");

        assert_eq!(
            status.len(),
            1,
            "should have all active notif node ids in list"
        );
        assert!(status.get(&node_id_test()).unwrap());
    }

    fn test_notification(bill_id: &BillId, payload: Option<Value>) -> Notification {
        Notification::new_bill_notification(
            bill_id,
            &node_id_test(),
            "test_notification",
            payload,
            NotificationLevel::Informational,
        )
    }

    fn test_payload() -> Value {
        json!({ "Some": "value", "for": 66, "testing": true })
    }

    fn test_general_notification() -> Notification {
        Notification {
            id: Uuid::new_v4().to_string(),
            node_id: Some(node_id_test()),
            notification_type: NotificationType::General,
            reference_id: Some("general".to_string()),
            description: "general desc".to_string(),
            datetime: Timestamp::now().to_datetime(),
            active: true,
            level: NotificationLevel::Informational,
            payload: None,
            event_id: None,
        }
    }
}
