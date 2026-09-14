use super::Result;
use crate::service::transport_service::{
    BlockTransportServiceApi, ContactTransportServiceApi, NotificationTransportServiceApi,
};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::blockchain::bill::{
        BitcreditBill,
        participant::{BillIdentParticipant, BillParticipant},
    },
    protocol::crypto::BcrKeys,
    protocol::event::ActionType,
    protocol::event::BillChainEvent,
};
use std::sync::Arc;

#[cfg(test)]
use mockall::automock;

/// Allows to sync and manage contacts with the remote transport network
#[allow(dead_code)]
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait TransportServiceApi: ServiceTraitBounds {
    /// Returns the block propagation service
    fn block_transport(&self) -> &Arc<dyn BlockTransportServiceApi>;

    /// Returns the contact sync and management service
    fn contact_transport(&self) -> &Arc<dyn ContactTransportServiceApi>;

    /// Returns the notification service
    fn notification_transport(&self) -> &Arc<dyn NotificationTransportServiceApi>;

    /// Connects to the underlying network
    async fn connect(&self);

    /// Sent when: A bill is signed by: Drawer
    /// Receiver: Payer, Action: AcceptBill
    /// Receiver: Payee, Action: CheckBill
    async fn send_bill_is_signed_event(&self, event: &BillChainEvent) -> Result<()>;

    /// Sent when: A bill is accepted by: Payer
    /// Receiver: Holder, Action: CheckBill
    async fn send_bill_is_accepted_event(&self, event: &BillChainEvent) -> Result<()>;

    /// Sent when: A bill is requested to be accepted, Sent by: Holder
    /// Receiver: Payer, Action: AcceptBill
    async fn send_request_to_accept_event(&self, event: &BillChainEvent) -> Result<()>;

    /// Sent when: A bill is requested to be paid, Sent by: Holder
    /// Receiver: Payer, Action: PayBill
    async fn send_request_to_pay_event(&self, event: &BillChainEvent) -> Result<()>;

    /// Sent when: A bill is endorsed by: Previous Holder
    /// Receiver: NewHolder, Action: CheckBill
    async fn send_bill_is_endorsed_event(&self, event: &BillChainEvent) -> Result<()>;

    /// Sent when: A bill is offered to be sold, Sent by: Holder
    /// Receiver: Buyer, Action: CheckBill (with buy page)
    async fn send_offer_to_sell_event(
        &self,
        event: &BillChainEvent,
        buyer: &BillParticipant,
    ) -> Result<()>;

    /// Sent when: A bill is sold by: Seller (old holder)
    /// Receiver: Buyer (new holder), Action: CheckBill
    async fn send_bill_is_sold_event(
        &self,
        event: &BillChainEvent,
        buyer: &BillParticipant,
    ) -> Result<()>;

    /// Sent when: A bill recourse was paid, by: Recourser (old holder)
    /// Receiver: Recoursee (new holder), Action: CheckBill
    async fn send_bill_recourse_paid_event(
        &self,
        event: &BillChainEvent,
        recoursee: &BillIdentParticipant,
    ) -> Result<()>;

    /// In case a participant rejects one of the 'request to' actions (e.g. request to accept,
    /// request to pay) we send this event to all bill participants. Will only send the event
    /// if the given action can be a rejected action.
    /// Arguments:
    /// * bill_id: The id of the bill affected
    /// * rejected_action: The action that was rejected
    /// * recipients: The list of recipients that should receive the notification
    async fn send_request_to_action_rejected_event(
        &self,
        event: &BillChainEvent,
        rejected_action: ActionType,
    ) -> Result<()>;

    /// In case an action was rejected or timed out a holder can request a recourse action
    /// from another participant in the chain. Will only send the event if the given action
    /// can be a recourse action.
    /// Arguments:
    /// * bill_id: The id of the bill affected
    /// * action: The action that should be performed via recourse. This will also be the action
    /// sent in the event given it can be a recourse action.
    /// * recipient: The recourse recipient that should perform the action
    async fn send_recourse_action_event(
        &self,
        event: &BillChainEvent,
        action: ActionType,
        recoursee: &BillIdentParticipant,
    ) -> Result<()>;

    /// Sent when: A bill is requested to be minted, Sent by: Holder
    /// Receiver: Mint, Action: CheckBill (with generate quote page)
    async fn send_request_to_mint_event(
        &self,
        sender_node_id: &NodeId,
        mint: &BillParticipant,
        bill: &BitcreditBill,
    ) -> Result<()>;

    /// Retry sending a queued message to the given node id
    async fn send_retry_messages(&self) -> Result<()>;

    /// Syncs historical events to newly added relays
    async fn sync_relays(&self) -> Result<()>;

    /// Retries failed relay sync attempts
    async fn retry_failed_syncs(&self) -> Result<()>;

    /// Adds a new identity (company keys) to the multi-identity client.
    /// This will also add a subscription for direct messages to this identity.
    async fn add_identity(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<()>;

    /// Queries historical private messages for a company identity and processes any
    /// bill chain invites found, constructing the bills in the local store.
    async fn process_company_historical_bill_invites(&self, company_id: &NodeId) -> Result<()>;

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

    /// Queries historical file metadata (kind:1063) events for a given file hash.
    /// Returns events that contain server URLs for the file.
    async fn query_file_metadata_events(
        &self,
        file_hash: &str,
        nostr_hash: &str,
    ) -> Result<Vec<nostr::event::Event>>;
}

#[cfg(test)]
use crate::service::transport_service::{
    MockBlockTransportServiceApi, MockContactTransportServiceApi,
    MockNotificationTransportServiceApi,
};

#[cfg(test)]
impl ServiceTraitBounds for MockTransportServiceApi {}

#[cfg(test)]
impl MockTransportServiceApi {
    /// None of the contained transports are used
    pub fn unused() -> Self {
        let block_transport = Arc::new(MockBlockTransportServiceApi::new());
        let contact_transport = Arc::new(MockContactTransportServiceApi::new());
        let notification_transport = Arc::new(MockNotificationTransportServiceApi::new());
        Self::new()
            .with_block_transport(block_transport)
            .with_contact_transport(contact_transport)
            .with_notification_transport(notification_transport)
    }

    pub fn with_block_transport(
        mut self,
        block_transport: Arc<dyn BlockTransportServiceApi>,
    ) -> Self {
        self.expect_block_transport().return_const(block_transport);
        self
    }

    pub fn with_contact_transport(
        mut self,
        contact_transport: Arc<dyn ContactTransportServiceApi>,
    ) -> Self {
        self.expect_contact_transport()
            .return_const(contact_transport);
        self
    }

    pub fn with_notification_transport(
        mut self,
        notification_transport: Arc<dyn NotificationTransportServiceApi>,
    ) -> Self {
        self.expect_notification_transport()
            .return_const(notification_transport);
        self
    }

    pub fn expect_on_block_transport(
        &mut self,
        expect: impl Fn(&mut MockBlockTransportServiceApi),
    ) {
        use crate::service::transport_service::MockBlockTransportServiceApi;
        let mut block = MockBlockTransportServiceApi::new();
        expect(&mut block);
        self.expect_block_transport().return_const(Arc::new(block));
    }

    pub fn expect_on_contact_transport(
        &mut self,
        expect: impl Fn(&mut MockContactTransportServiceApi),
    ) {
        use crate::service::transport_service::MockContactTransportServiceApi;
        let mut contact = MockContactTransportServiceApi::new();
        expect(&mut contact);
        self.expect_contact_transport()
            .return_const(Arc::new(contact));
    }

    pub fn expect_on_notification_transport(
        &mut self,
        expect: impl Fn(&mut MockNotificationTransportServiceApi),
    ) {
        use crate::service::transport_service::MockNotificationTransportServiceApi;
        let mut notification = MockNotificationTransportServiceApi::new();
        expect(&mut notification);
        self.expect_notification_transport()
            .return_const(Arc::new(notification));
    }
}
