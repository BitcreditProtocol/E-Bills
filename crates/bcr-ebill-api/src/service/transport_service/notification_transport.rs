use super::Result;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_common::wire::quotes::ApplicantActionProjection;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    application::notification::{Notification, NotificationLevel},
    protocol::Sum,
    protocol::blockchain::bill::participant::BillParticipant,
    protocol::event::ActionType,
    protocol::event::{BillChainEventPayload, BillEventType, Event},
};
use bcr_ebill_persistence::notification::NotificationFilter;
use std::collections::HashMap;

#[cfg(test)]
use mockall::automock;

/// Allows to sync and manage contacts with the remote transport network
#[allow(dead_code)]
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait NotificationTransportServiceApi: ServiceTraitBounds {
    /// Returns filtered client notifications
    async fn get_client_notifications(
        &self,
        filter: NotificationFilter,
    ) -> Result<Vec<Notification>>;

    /// Marks the notification with given id as done
    async fn mark_notification_as_done(&self, notification_id: &str) -> Result<()>;

    /// Returns the active bill notification for the given bill id
    async fn get_active_bill_notification(&self, bill_id: &BillId) -> Option<Notification>;

    async fn get_active_bill_notifications(
        &self,
        bill_ids: &[BillId],
    ) -> HashMap<BillId, Notification>;

    async fn get_active_notification_status_for_node_ids(
        &self,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, bool>>;

    /// Creates a local bill notification for the given node without sending Nostr events.
    /// Marks any existing active bill notification as done and pushes to connected clients.
    async fn create_local_bill_notification(
        &self,
        node_id: &NodeId,
        bill_id: &BillId,
        event_type: BillEventType,
        action_type: Option<ActionType>,
        sum: Option<Sum>,
    ) -> Result<()>;

    /// Reconciles the mint's authoritative applicant-action projection into local persistence.
    /// This does not publish a bill-chain or Nostr event.
    async fn reconcile_quote_applicant_action_notification(
        &self,
        node_id: &NodeId,
        bill_id: &BillId,
        mint_request_id: uuid::Uuid,
        applicant_action: Option<ApplicantActionProjection>,
    ) -> Result<()>;

    /// Creates a general (non-bill, non-company) notification for the given node.
    /// Used for system-level notifications like "save your seed phrase".
    async fn create_general_notification(
        &self,
        node_id: &NodeId,
        description: &str,
        reference_id: Option<String>,
        level: NotificationLevel,
    ) -> Result<()>;

    /// In case a participant did not perform an action (e.g. request to accept, request
    /// to pay) in time we notify all bill participants about the timed out action. Will
    /// only send the event if the given action can be a timed out action.
    /// Arguments:
    /// * bill_id: The id of the bill affected
    /// * timed_out_action: The action that has timed out
    /// * recipients: The list of recipients that should receive the notification
    async fn send_request_to_action_timed_out_event(
        &self,
        sender_node_id: &NodeId,
        bill_id: &BillId,
        sum: Option<Sum>,
        timed_out_action: ActionType,
        recipients: Vec<BillParticipant>,
        holder: &NodeId,
        drawee: &NodeId,
        recoursee: &Option<NodeId>,
    ) -> Result<()>;

    /// Returns whether a notification was already sent for the given bill id and action
    async fn check_bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action: ActionType,
    ) -> Result<bool>;

    /// Stores that a notification was sent for the given bill id and action
    async fn mark_bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action: ActionType,
    ) -> Result<()>;

    /// Fetch email notifications preferences link for the currently selected identity
    async fn get_email_notifications_preferences_link(&self, node_id: &NodeId) -> Result<url::Url>;

    /// Attempts to send an email notification for an event to the receiver
    /// if the receiver does not have email notifications enabled, the relay
    /// ignores the request and returns a quick 200 OK.
    async fn send_email_notification(
        &self,
        sender: &NodeId,
        receiver: &NodeId,
        event: &Event<BillChainEventPayload>,
    );
}

#[cfg(test)]
impl ServiceTraitBounds for MockNotificationTransportServiceApi {}
