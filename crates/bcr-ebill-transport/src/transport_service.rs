use async_trait::async_trait;
use std::collections::HashMap;

use bcr_common::core::NodeId;
use bcr_ebill_api::service::transport_service::{
    BlockTransportServiceApi, ContactTransportServiceApi, NotificationTransportServiceApi,
    TransportServiceApi,
};
use bcr_ebill_core::protocol::blockchain::bill::BitcreditBill;
use bcr_ebill_core::protocol::blockchain::bill::participant::{
    BillIdentParticipant, BillParticipant,
};
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bcr_ebill_core::protocol::event::{BillChainEvent, BillChainEventPayload, Event};

use super::handler::NotificationHandlerApi;
use super::nostr_transport::NostrTransportService;
use bcr_ebill_api::service::transport_service::Result;
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::protocol::event::{ActionType, BillEventType, EventType};
use log::{debug, error, info, warn};
use std::sync::Arc;

pub struct TransportService {
    nostr_transport: Arc<NostrTransportService>,
    notification_transport_service: Arc<dyn NotificationTransportServiceApi>,
    contact_transport_service: Arc<dyn ContactTransportServiceApi>,
    block_transport_service: Arc<dyn BlockTransportServiceApi>,
    bill_invite_handler: Arc<dyn NotificationHandlerApi>,
}

impl TransportService {
    pub fn new(
        nostr_transport: Arc<NostrTransportService>,
        notification_transport_service: Arc<dyn NotificationTransportServiceApi>,
        contact_transport_service: Arc<dyn ContactTransportServiceApi>,
        block_transport_service: Arc<dyn BlockTransportServiceApi>,
        bill_invite_handler: Arc<dyn NotificationHandlerApi>,
    ) -> Self {
        Self {
            nostr_transport,
            notification_transport_service,
            contact_transport_service,
            block_transport_service,
            bill_invite_handler,
        }
    }
}

impl ServiceTraitBounds for TransportService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl TransportServiceApi for TransportService {
    fn block_transport(&self) -> &Arc<dyn BlockTransportServiceApi> {
        &self.block_transport_service
    }

    fn contact_transport(&self) -> &Arc<dyn ContactTransportServiceApi> {
        &self.contact_transport_service
    }

    #[doc = " Returns the notification service"]
    fn notification_transport(&self) -> &Arc<dyn NotificationTransportServiceApi> {
        &self.notification_transport_service
    }

    async fn connect(&self) {
        self.nostr_transport.connect().await;
    }

    async fn add_identity(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<()> {
        self.nostr_transport.add_identity(node_id, keys).await
    }

    async fn send_bill_is_signed_event(&self, event: &BillChainEvent) -> Result<()> {
        let all_events = event.generate_messages(BillEventType::BillSigned);

        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;

        let payee = event.bill.payee.node_id();
        let drawee = event.bill.drawee.node_id.clone();
        let drawer = event.bill.drawer.node_id.clone();

        if let Some(payee_event) = all_events.get(&payee) {
            // No self-notification: don't email payee if drawer is payee (unless drawer == drawee)
            if drawer != payee || drawer == drawee {
                self.notification_transport_service
                    .send_email_notification(&event.sender(), &payee, payee_event)
                    .await;
            }
        }
        if let Some(drawee_event) = all_events.get(&drawee) {
            // No self-notification: don't email drawee if drawer is drawee (unless drawer == payee)
            if drawer != drawee || drawer == payee {
                self.notification_transport_service
                    .send_email_notification(&event.sender(), &drawee, drawee_event)
                    .await;
            }
        }

        Ok(())
    }

    async fn send_bill_is_accepted_event(&self, event: &BillChainEvent) -> Result<()> {
        let all_events = event.generate_messages(BillEventType::BillAccepted);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        let holder = event
            .bill
            .endorsee
            .as_ref()
            .map(|e| e.node_id())
            .unwrap_or_else(|| event.bill.payee.node_id());
        if let Some(holder_event) = all_events.get(&holder) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &holder, holder_event)
                .await;
        }
        Ok(())
    }

    async fn send_request_to_accept_event(&self, event: &BillChainEvent) -> Result<()> {
        let all_events = event.generate_messages(BillEventType::BillAcceptanceRequested);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        let drawee = event.bill.drawee.node_id.clone();
        if let Some(drawee_event) = all_events.get(&drawee) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &drawee, drawee_event)
                .await;
        }
        Ok(())
    }

    async fn send_request_to_pay_event(&self, event: &BillChainEvent) -> Result<()> {
        let all_events = event.generate_messages(BillEventType::BillPaymentRequested);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        let drawee = event.bill.drawee.node_id.clone();
        if let Some(drawee_event) = all_events.get(&drawee) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &drawee, drawee_event)
                .await;
        }
        Ok(())
    }

    async fn send_bill_is_endorsed_event(&self, event: &BillChainEvent) -> Result<()> {
        let all_events = event.generate_messages(BillEventType::BillEndorsed);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        if let Some(ref endorsee) = event.bill.endorsee {
            let endorsee_node_id = endorsee.node_id();
            if let Some(endorsee_event) = all_events.get(&endorsee_node_id) {
                self.notification_transport_service
                    .send_email_notification(&event.sender(), &endorsee_node_id, endorsee_event)
                    .await;
            }
        }
        Ok(())
    }

    async fn send_offer_to_sell_event(
        &self,
        event: &BillChainEvent,
        buyer: &BillParticipant,
    ) -> Result<()> {
        let all_events = event.generate_offer_to_sell_messages(buyer);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        let buyer_node_id = buyer.node_id();
        if let Some(buyer_event) = all_events.get(&buyer_node_id) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &buyer_node_id, buyer_event)
                .await;
        }
        Ok(())
    }

    async fn send_bill_is_sold_event(
        &self,
        event: &BillChainEvent,
        buyer: &BillParticipant,
    ) -> Result<()> {
        let all_events = event.generate_bill_sold_messages(buyer);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        let buyer_node_id = buyer.node_id();
        if let Some(buyer_event) = all_events.get(&buyer_node_id) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &buyer_node_id, buyer_event)
                .await;
        }
        Ok(())
    }

    async fn send_bill_recourse_paid_event(
        &self,
        event: &BillChainEvent,
        recoursee: &BillIdentParticipant,
    ) -> Result<()> {
        let all_events = event.generate_action_messages(
            HashMap::from_iter(vec![(
                recoursee.node_id.clone(),
                (BillEventType::BillRecoursePaid, ActionType::CheckBill),
            )]),
            None,
            None,
        );
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        self.nostr_transport
            .send_all_bill_events(&event.sender(), &all_events)
            .await?;
        if let Some(recoursee_event) = all_events.get(&recoursee.node_id) {
            self.notification_transport_service
                .send_email_notification(&event.sender(), &recoursee.node_id, recoursee_event)
                .await;
        }
        Ok(())
    }

    async fn send_request_to_mint_event(
        &self,
        sender_node_id: &NodeId,
        mint: &BillParticipant,
        bill: &BitcreditBill,
    ) -> Result<()> {
        let event = Event::new_bill(BillChainEventPayload {
            event_type: BillEventType::BillMintingRequested,
            bill_id: bill.id.clone(),
            action_type: Some(ActionType::CheckBill),
            sum: Some(bill.sum.clone()),
            sender_node_id: Some(sender_node_id.clone()),
            sender_name: None,
        });
        let node = self.nostr_transport.get_node_transport(sender_node_id);
        node.send_private_event(sender_node_id, mint, event.clone().try_into()?)
            .await?;
        // Only send email to mint
        self.notification_transport_service
            .send_email_notification(sender_node_id, &mint.node_id(), &event)
            .await;
        Ok(())
    }

    async fn send_request_to_action_rejected_event(
        &self,
        event: &BillChainEvent,
        rejected_action: ActionType,
    ) -> Result<()> {
        let all_events = event.generate_rejected_messages(rejected_action);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        if !all_events.is_empty() {
            self.nostr_transport
                .send_all_bill_events(&event.sender(), &all_events)
                .await?;
            let holder = event
                .bill
                .endorsee
                .as_ref()
                .map(|e| e.node_id())
                .unwrap_or_else(|| event.bill.payee.node_id());
            if let Some(holder_event) = all_events.get(&holder) {
                self.notification_transport_service
                    .send_email_notification(&event.sender(), &holder, holder_event)
                    .await;
            }
        }
        Ok(())
    }

    async fn send_recourse_action_event(
        &self,
        event: &BillChainEvent,
        action: ActionType,
        recoursee: &BillIdentParticipant,
    ) -> Result<()> {
        let all_events = event.generate_recourse_messages(action, recoursee);
        self.block_transport_service
            .send_bill_chain_events(event.clone())
            .await?;
        if !all_events.is_empty() {
            self.nostr_transport
                .send_all_bill_events(&event.sender(), &all_events)
                .await?;
            if let Some(recoursee_event) = all_events.get(&recoursee.node_id) {
                self.notification_transport_service
                    .send_email_notification(&event.sender(), &recoursee.node_id, recoursee_event)
                    .await;
            }
        }
        Ok(())
    }

    async fn send_retry_messages(&self) -> Result<()> {
        self.nostr_transport.send_retry_messages().await
    }

    async fn sync_relays(&self) -> Result<()> {
        self.nostr_transport
            .get_first_transport()
            .sync_relays()
            .await
    }

    async fn retry_failed_syncs(&self) -> Result<()> {
        self.nostr_transport
            .get_first_transport()
            .retry_failed_syncs()
            .await
    }

    async fn process_company_historical_bill_invites(&self, company_id: &NodeId) -> Result<()> {
        let node = self.nostr_transport.get_first_transport();
        if !node.has_local_signer(company_id) {
            warn!("Try to process_company_historical_bill_invites without signer in transport");
            return Ok(());
        };
        let events = node
            .resolve_events(
                nostr::filter::Filter::new()
                    .pubkey(company_id.npub())
                    .kinds(vec![nostr::event::Kind::GiftWrap])
                    .since(nostr::types::Timestamp::zero()),
            )
            .await?;
        info!(
            "found {} private events for company {} historical bill invite processing",
            events.len(),
            company_id
        );

        for event in events {
            match node.try_decrypt_private_event(&event).await {
                Ok(Some((_recipient_id, envelope, sender))) => {
                    if envelope.event_type == EventType::BillChainInvite
                        && let Err(e) = self
                            .bill_invite_handler
                            .handle_event(envelope, company_id, Some(sender), Some(Box::new(event)))
                            .await
                    {
                        error!(
                            "Failed to process historical bill invite for company {}: {}",
                            company_id, e
                        );
                    }
                }
                Ok(None) => {
                    debug!(
                        "Could not decrypt event {} for company {}",
                        event.id, company_id
                    );
                }
                Err(e) => {
                    error!(
                        "Error decrypting event {} for company {}: {}",
                        event.id, company_id, e
                    );
                }
            }
        }
        Ok(())
    }

    async fn publish_file_metadata(
        &self,
        node_id: &NodeId,
        plaintext_hash: &str,
        encrypted_hash: &str,
        server_urls: Vec<url::Url>,
        mime_type: Option<String>,
    ) -> Result<()> {
        self.nostr_transport
            .get_first_transport()
            .publish_file_metadata(
                node_id,
                plaintext_hash,
                encrypted_hash,
                server_urls,
                mime_type,
            )
            .await
    }

    async fn query_file_metadata_events(
        &self,
        file_hash: &str,
        nostr_hash: &str,
    ) -> Result<Vec<nostr::event::Event>> {
        self.nostr_transport
            .get_first_transport()
            .query_file_metadata_events(file_hash, nostr_hash)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::Error;
    use crate::test_utils::{
        MockBlockTransportService, MockContactTransportService, MockNotificationTransportService,
        get_nostr_transport, signed_identity_proof_test,
    };
    use bcr_ebill_core::application::contact::Contact;
    use bcr_ebill_core::protocol::Timestamp;
    use bcr_ebill_core::protocol::blockchain::Blockchain;
    use bcr_ebill_core::protocol::blockchain::bill::block::{
        BillAcceptBlockData, BillOfferToSellBlockData, BillParticipantBlockData,
        BillPaymentBlockData, BillRecourseBlockData, BillRequestToAcceptBlockData,
        BillRequestToPayBlockData,
    };
    use bcr_ebill_core::protocol::blockchain::bill::{BillBlock, BillBlockchain};
    use bcr_ebill_core::protocol::constants::{
        ACCEPT_DEADLINE_SECONDS, DAY_IN_SECS, PAYMENT_DEADLINE_SECONDS,
    };
    use bcr_ebill_core::protocol::event::{ChainInvite, EventEnvelope, EventType};
    use bcr_ebill_core::{
        protocol::Email, protocol::Result, protocol::Sum, protocol::crypto::BcrKeys,
    };
    use bcr_ebill_persistence::nostr::NostrQueuedMessage;
    use bitcoin::base58;
    use mockall::predicate::eq;
    use nostr::event::FinalizeEvent;
    use std::sync::Arc;

    use crate::test_utils::{
        MockContactStore, MockNostrChainEventStore, MockNostrContactStore,
        MockNostrQueuedMessageStore, MockNotificationJsonTransport, bill_id_test, empty_address,
        get_baseline_identity, get_genesis_chain, init_test_cfg, node_id_test, node_id_test_other,
        node_id_test_other2, private_key_test, valid_payment_address_testnet,
    };

    use super::super::test_utils::{
        MockNotificationHandler, get_identity_public_data, get_test_bitcredit_bill,
    };
    use super::*;

    fn check_chain_payload(event: &EventEnvelope, bill_event_type: BillEventType) -> bool {
        let valid_event_type = event.event_type == EventType::Bill;
        let event: Result<Event<BillChainEventPayload>> = event.clone().try_into();
        if let Ok(event) = event {
            valid_event_type && event.data.event_type == bill_event_type
        } else {
            false
        }
    }

    fn get_test_nostr_event() -> nostr::event::Event {
        nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test broadcast message")
            .finalize(&nostr::key::Keys::generate())
            .expect("Could not create nostr test event")
    }

    fn get_mocks() -> (
        MockNotificationJsonTransport,
        MockContactStore,
        MockNostrContactStore,
        MockNostrQueuedMessageStore,
        MockNostrChainEventStore,
        MockNotificationTransportService,
        MockContactTransportService,
        MockBlockTransportService,
    ) {
        let mut mock_transport = MockNotificationJsonTransport::new();
        // Set default expectation for has_local_signer to return false for any node_id
        // Tests can override this expectation as needed
        mock_transport
            .expect_has_local_signer()
            .returning(|_| false);

        (
            mock_transport,
            MockContactStore::new(),
            MockNostrContactStore::new(),
            MockNostrQueuedMessageStore::new(),
            MockNostrChainEventStore::new(),
            MockNotificationTransportService::new(),
            MockContactTransportService::new(),
            MockBlockTransportService::new(),
        )
    }
    fn get_transport(
        mock_transport: MockNotificationJsonTransport,
        contact_store: MockContactStore,
        nostr_contact_store: MockNostrContactStore,
        queued_message_store: MockNostrQueuedMessageStore,
        chain_events: MockNostrChainEventStore,
        mock_notification_transport: MockNotificationTransportService,
        mock_contact_transport: MockContactTransportService,
        mock_block_transport: MockBlockTransportService,
    ) -> TransportService {
        get_transport_with_handler(
            mock_transport,
            contact_store,
            nostr_contact_store,
            queued_message_store,
            chain_events,
            mock_notification_transport,
            mock_contact_transport,
            mock_block_transport,
            Arc::new(MockNotificationHandler::new()),
        )
    }

    fn get_transport_with_handler(
        mock_transport: MockNotificationJsonTransport,
        contact_store: MockContactStore,
        nostr_contact_store: MockNostrContactStore,
        queued_message_store: MockNostrQueuedMessageStore,
        chain_events: MockNostrChainEventStore,
        mock_notification_transport: MockNotificationTransportService,
        mock_contact_transport: MockContactTransportService,
        mock_block_transport: MockBlockTransportService,
        handler: Arc<dyn NotificationHandlerApi>,
    ) -> TransportService {
        TransportService::new(
            Arc::new(get_nostr_transport(
                mock_transport,
                contact_store,
                nostr_contact_store,
                queued_message_store,
                chain_events,
            )),
            Arc::new(mock_notification_transport),
            Arc::new(mock_contact_transport),
            Arc::new(mock_block_transport),
            handler,
        )
    }

    fn expect_service<T>(
        expect: impl Fn(
            &mut MockNotificationJsonTransport,
            &mut MockContactStore,
            &mut MockNostrContactStore,
            &mut MockNostrQueuedMessageStore,
            &mut MockNostrChainEventStore,
            &mut MockNotificationTransportService,
            &mut MockContactTransportService,
            &mut MockBlockTransportService,
        ) -> T,
    ) -> (TransportService, T) {
        let (
            mut transport,
            mut contact_store,
            mut nostr_contact_store,
            mut queued_message_store,
            mut chain_events,
            mut notification_transport,
            mut contact_transport,
            mut block_transport,
        ) = get_mocks();

        let value = expect(
            &mut transport,
            &mut contact_store,
            &mut nostr_contact_store,
            &mut queued_message_store,
            &mut chain_events,
            &mut notification_transport,
            &mut contact_transport,
            &mut block_transport,
        );

        (
            get_transport(
                transport,
                contact_store,
                nostr_contact_store,
                queued_message_store,
                chain_events,
                notification_transport,
                contact_transport,
                block_transport,
            ),
            value,
        )
    }

    #[tokio::test]
    async fn test_connect() {
        init_test_cfg();
        let mut mock_transport = MockNotificationJsonTransport::new();

        // call connect on the inner transport
        mock_transport.expect_connect().returning(|| Ok(()));

        let service = NostrTransportService::new(
            Arc::new(mock_transport),
            Arc::new(MockContactStore::new()),
            Arc::new(MockNostrContactStore::new()),
            Arc::new(MockNostrQueuedMessageStore::new()),
            Arc::new(MockNostrChainEventStore::new()),
            vec![url::Url::parse("ws://test.relay").unwrap()],
        );

        service.connect().await;
    }

    #[tokio::test]
    async fn test_send_request_to_action_rejected_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        let (service, _) = expect_service(
            move |transport, contact_store, _, _, _, notification_transport, _, block_transport| {
                let buyer = buyer.clone();
                let payer = payer.clone();
                let payee = payee.clone();
                contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&buyer))));
                contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payer))));
                contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payee))));

                // expect to send payment rejected event to all recipients (except payer)
                transport
                    .expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillPaymentRejected))
                    .returning(|_, _, _| Ok(()))
                    .times(2);

                // expect to send acceptance rejected event to all recipients (except payer)
                transport
                    .expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillAcceptanceRejected))
                    .returning(|_, _, _| Ok(()))
                    .times(2);

                // expect to send buying rejected event to all recipients (except payer)
                transport
                    .expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillBuyingRejected))
                    .returning(|_, _, _| Ok(()))
                    .times(2);

                // Recourse rejection goes to prior holders and recourser only; this test chain has no previous holders
                transport
                    .expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillRecourseRejected))
                    .returning(|_, _, _| Ok(()))
                    .times(1);

                block_transport
                    .expect_send_bill_chain_events()
                    .returning(|_| Ok(()))
                    .times(4);

                notification_transport
                    .expect_send_email_notification()
                    .returning(|_, _, _| ())
                    .times(4);

                // this is only required for the test as it contains an invite block so it tries to send an
                // invite to new participants as well and the test data doesn't have them all.
                transport
                    .expect_send_private_event()
                    .withf(|_, _, e| e.event_type == EventType::BillChainInvite)
                    .returning(|_, _, _| Ok(()));
            },
        );

        service
            .send_request_to_action_rejected_event(&event, ActionType::PayBill)
            .await
            .expect("failed to send event");

        service
            .send_request_to_action_rejected_event(&event, ActionType::AcceptBill)
            .await
            .expect("failed to send event");

        service
            .send_request_to_action_rejected_event(&event, ActionType::BuyBill)
            .await
            .expect("failed to send event");

        service
            .send_request_to_action_rejected_event(&event, ActionType::RecourseBill)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_request_to_action_rejected_does_not_send_non_rejectable_action() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        let (service, _) =
            expect_service(|mock, mock_contact_store, _, _, _, _, _, block_transport| {
                // no participant should receive events
                mock_contact_store.expect_get().never();

                // expect to not send rejected event for non rejectable actions
                mock.expect_send_private_event().never();

                block_transport
                    .expect_send_bill_chain_events()
                    .returning(|_| Ok(()))
                    .times(1);
            });

        service
            .send_request_to_action_rejected_event(&event, ActionType::CheckBill)
            .await
            .expect("failed to send event");
    }

    fn as_contact(id: &BillIdentParticipant) -> Contact {
        Contact {
            t: id.t.clone(),
            node_id: id.node_id.clone(),
            name: id.name.to_owned(),
            email: id.email.clone(),
            postal_address: Some(id.postal_address.clone()),
            nostr_relays: id.nostr_relays.clone(),
            identification_number: None,
            avatar_file: None,
            proof_document_file: None,
            date_of_birth_or_registration: None,
            country_of_birth_or_registration: None,
            city_of_birth_or_registration: None,
            is_logical: false,
            mint_url: None,
        }
    }

    #[tokio::test]
    async fn test_send_recourse_action_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        let (service, _) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let buyer = buyer.clone();
                let payer = payer.clone();
                let payee = payee.clone();
                // participants should receive events
                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&buyer))));
                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payee))));
                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payer))));

                // expect to send payment recourse event to recoursee only
                mock.expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillPaymentRecourse))
                    .returning(|_, _, _| Ok(()))
                    .times(1);

                // expect to send acceptance recourse event to recoursee only
                mock.expect_send_private_event()
                    .withf(|_, _, e| check_chain_payload(e, BillEventType::BillAcceptanceRecourse))
                    .returning(|_, _, _| Ok(()))
                    .times(1);

                block_transport
                    .expect_send_bill_chain_events()
                    .returning(|_| Ok(()))
                    .times(2);

                notification_transport
                    .expect_send_email_notification()
                    .returning(|_, _, _| ())
                    .times(2);

                mock.expect_send_private_event()
                    .withf(move |_, _, e| {
                        let r: bcr_ebill_core::protocol::Result<Event<ChainInvite>> =
                            e.clone().try_into();
                        r.is_ok()
                    })
                    .returning(|_, _, _| Ok(()));
            },
        );

        service
            .send_recourse_action_event(&event, ActionType::PayBill, &buyer)
            .await
            .expect("failed to send event");

        service
            .send_recourse_action_event(&event, ActionType::AcceptBill, &buyer)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_recourse_action_event_does_not_send_non_recourse_action() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        let (service, _) = expect_service(|mock, _, _, _, _, _, _, block_transport| {
            // expect not to send non recourse event
            mock.expect_send_private_event().never();
            block_transport
                .expect_send_bill_chain_events()
                .returning(|_| Ok(()))
                .times(1);
        });

        service
            .send_recourse_action_event(&event, ActionType::CheckBill, &payer)
            .await
            .expect("failed to send event");
    }

    fn setup_chain_expectation(
        participants: Vec<(BillIdentParticipant, BillEventType, Option<ActionType>)>,
        bill: &BitcreditBill,
        chain: &BillBlockchain,
        new_blocks: bool,
        mock_contact_store: &mut MockContactStore,
        mock: &mut MockNotificationJsonTransport,
        mock_block_transport: &mut MockBlockTransportService,
        mock_notification_transport: &mut MockNotificationTransportService,
    ) -> BillChainEvent {
        mock_notification_transport
            .expect_send_email_notification()
            .returning(|_, _, _| ());

        for p in participants.into_iter() {
            let clone1 = p.clone();
            mock_contact_store
                .expect_get()
                .with(eq(clone1.0.node_id.clone()))
                .returning(move |_| Ok(Some(as_contact(&clone1.0))));

            let clone2 = p.clone();
            mock.expect_send_private_event()
                .withf(move |_, r, e| {
                    let part = clone2.clone();
                    let valid_node_id = r.node_id() == part.0.node_id;
                    let event_result: bcr_ebill_core::protocol::Result<
                        Event<BillChainEventPayload>,
                    > = e.clone().try_into();
                    if let Ok(event) = event_result {
                        let valid_event_type = event.data.event_type == part.1;
                        valid_node_id && valid_event_type && event.data.action_type == part.2
                    } else {
                        false
                    }
                })
                .returning(|_, _, _| Ok(()));

            mock.expect_send_private_event()
                .withf(move |_, _, e| {
                    let r: bcr_ebill_core::protocol::Result<Event<ChainInvite>> =
                        e.clone().try_into();
                    r.is_ok()
                })
                .returning(|_, _, _| Ok(()));
        }
        mock_block_transport
            .expect_send_bill_chain_events()
            .returning(|_| Ok(()))
            .once();

        BillChainEvent::new(
            bill,
            chain,
            &BcrKeys::from_private_key(&private_key_test()),
            new_blocks,
            &node_id_test(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_send_bill_is_signed_event() {
        init_test_cfg();
        // given a payer and payee with a new bill
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));
        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (
                            payer,
                            BillEventType::BillSigned,
                            Some(ActionType::AcceptBill),
                        ),
                        (
                            payee,
                            BillEventType::BillSigned,
                            Some(ActionType::CheckBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_bill_is_signed_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_bill_is_accepted_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_accept(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillAcceptBlockData {
                accepter: payer.clone().into(),
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (
                            payee,
                            BillEventType::BillAccepted,
                            Some(ActionType::CheckBill),
                        ),
                        (
                            payer,
                            BillEventType::BillAccepted,
                            Some(ActionType::CheckBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_bill_is_accepted_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_request_to_accept_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_request_to_accept(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payee.clone().into()),
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: timestamp + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (payee, BillEventType::BillBlock, None),
                        (
                            payer,
                            BillEventType::BillAcceptanceRequested,
                            Some(ActionType::AcceptBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_request_to_accept_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_request_to_pay_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_request_to_pay(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillRequestToPayBlockData {
                requester: BillParticipantBlockData::Ident(payee.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: bill.sum.clone(),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * PAYMENT_DEADLINE_SECONDS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (payee, BillEventType::BillBlock, None),
                        (
                            payer,
                            BillEventType::BillPaymentRequested,
                            Some(ActionType::PayBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_request_to_pay_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_bill_is_endorsed_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let endorsee = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("endorsee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, Some(&endorsee));
        let chain = get_genesis_chain(Some(bill.clone()));

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                let endorsee = endorsee.clone();
                setup_chain_expectation(
                    vec![
                        (payee, BillEventType::BillBlock, None),
                        (payer, BillEventType::BillBlock, None),
                        (
                            endorsee,
                            BillEventType::BillAcceptanceRequested,
                            Some(ActionType::AcceptBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    false,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_bill_is_endorsed_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_offer_to_sell_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (payee, BillEventType::BillBlock, None),
                        (payer, BillEventType::BillBlock, None),
                        (
                            buyer.clone(),
                            BillEventType::BillSellOffered,
                            Some(ActionType::CheckBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_offer_to_sell_event(&event, &BillParticipant::Ident(buyer))
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_bill_is_sold_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let buyer = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("buyer@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_offer_to_sell(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillOfferToSellBlockData {
                seller: BillParticipantBlockData::Ident(payee.clone().into()),
                buyer: BillParticipantBlockData::Ident(buyer.clone().into()),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(100).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: timestamp + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (
                            payee.clone(),
                            BillEventType::BillSold,
                            Some(ActionType::CheckBill),
                        ),
                        (
                            buyer.clone(),
                            BillEventType::BillSold,
                            Some(ActionType::CheckBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_bill_is_sold_event(&event, &BillParticipant::Ident(buyer))
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_bill_recourse_paid_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let recoursee = get_identity_public_data(
            &node_id_test_other2(),
            &Email::new("recoursee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_recourse(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillRecourseBlockData {
                recourser: BillParticipant::Ident(payee.clone()).into(),
                recoursee: recoursee.clone().into(),
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, event) = expect_service(
            |mock, mock_contact_store, _, _, _, notification_transport, _, block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();
                setup_chain_expectation(
                    vec![
                        (payee, BillEventType::BillBlock, None),
                        (payer, BillEventType::BillBlock, None),
                        (
                            recoursee.clone(),
                            BillEventType::BillRecoursePaid,
                            Some(ActionType::CheckBill),
                        ),
                    ],
                    &bill,
                    &chain,
                    true,
                    mock_contact_store,
                    mock,
                    block_transport,
                    notification_transport,
                )
            },
        );

        service
            .send_bill_recourse_paid_event(&event, &recoursee)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_request_to_mint_event() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let timestamp = Timestamp::now();
        let keys = get_baseline_identity().key_pair;
        let block = BillBlock::create_block_for_accept(
            bill.id.to_owned(),
            chain.get_latest_block(),
            &BillAcceptBlockData {
                accepter: payer.clone().into(),
                signatory: None,
                signing_timestamp: timestamp,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &keys,
            None,
            &keys,
            timestamp,
        )
        .unwrap();

        chain.try_add_block(block);

        let (service, _) = expect_service(|mock, _, _, _, _, notification_transport, _, _| {
            mock.expect_send_private_event()
                .returning(|_, _, _| Ok(()))
                .once();
            notification_transport
                .expect_send_email_notification()
                .returning(|_, _, _| ());
        });

        service
            .send_request_to_mint_event(
                &node_id_test(),
                &BillParticipant::Ident(payee.clone()),
                &bill,
            )
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_retry_messages_success() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let node_id = node_id_test_other();
                let message_id = "test_message_id";
                let sender_id = node_id_test();
                let payload = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );
                let queued_message = NostrQueuedMessage {
                    id: message_id.to_string(),
                    sender_id: sender_id.to_owned(),
                    recipient: Some(node_id.to_owned()),
                    payload: payload.clone(),
                };

                let identity = get_identity_public_data(
                    &node_id,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity))));

                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Ok(()));

                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message.clone()]))
                    .once();
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]));
                mock_queue
                    .expect_succeed_retry()
                    .with(eq(message_id))
                    .returning(|_| Ok(()));
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_send_failure() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let node_id = node_id_test_other();
                let message_id = "test_message_id";
                let sender_id = node_id_test();
                let payload = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );

                let queued_message = NostrQueuedMessage {
                    id: message_id.to_string(),
                    sender_id: sender_id.to_owned(),
                    recipient: Some(node_id.to_owned()),
                    payload: payload.clone(),
                };

                let identity = get_identity_public_data(
                    &node_id,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                // Set up mocks
                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity))));

                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Err(Error::Network("Failed to send".to_string())));

                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message.clone()]))
                    .once();
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]));
                mock_queue
                    .expect_fail_retry()
                    .with(eq(message_id))
                    .returning(|_| Ok(()));
            },
        );
        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_multiple_messages() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let node_id1 = node_id_test_other();
                let sender_id = node_id_test();
                let node_id2 = node_id_test_other2();
                let message_id1 = "test_message_id_1";
                let message_id2 = "test_message_id_2";

                let payload1 = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );

                let payload2 = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );

                let queued_message1 = NostrQueuedMessage {
                    id: message_id1.to_string(),
                    sender_id: sender_id.to_owned(),
                    recipient: Some(node_id1.to_owned()),
                    payload: payload1.clone(),
                };

                let queued_message2 = NostrQueuedMessage {
                    id: message_id2.to_string(),
                    sender_id: sender_id.to_owned(),
                    recipient: Some(node_id2.to_owned()),
                    payload: payload2.clone(),
                };

                let identity1 = get_identity_public_data(
                    &node_id1,
                    &Email::new("test1@example.com").unwrap(),
                    vec![],
                );
                let identity2 = get_identity_public_data(
                    &node_id2,
                    &Email::new("test2@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity1))));
                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity2))));

                // First message succeeds, second fails
                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Ok(()))
                    .times(1);
                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Err(Error::Network("Failed to send".to_string())))
                    .times(1);

                // Return first message, then second message
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message1.clone()]))
                    .times(1);
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message2.clone()]))
                    .times(1);
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]))
                    .times(1);

                mock_queue
                    .expect_succeed_retry()
                    .with(eq(message_id1))
                    .returning(|_| Ok(()));
                mock_queue
                    .expect_fail_retry()
                    .with(eq(message_id2))
                    .returning(|_| Ok(()));
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_invalid_payload() {
        init_test_cfg();

        let (service, _) = expect_service(|_, _, _, mock_queue, _, _, _, _| {
            let node_id = node_id_test_other();
            let message_id = "test_message_id";
            let sender = node_id_test();
            // Invalid payload that can't be deserialized to EventEnvelope
            let invalid_payload = base58::encode(&borsh::to_vec(&"invalid data").unwrap());

            let queued_message = NostrQueuedMessage {
                id: message_id.to_string(),
                sender_id: sender.to_owned(),
                recipient: Some(node_id.to_owned()),
                payload: invalid_payload,
            };

            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(move |_| Ok(vec![queued_message.clone()]))
                .times(1);
            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(|_| Ok(vec![]))
                .times(1);
            mock_queue
                .expect_fail_retry()
                .with(eq(message_id.to_string()))
                .returning(|_| Ok(()))
                .times(1);
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_fail_retry_error() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let node_id = node_id_test_other();
                let message_id = "test_message_id";
                let sender = node_id_test();
                let payload = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );

                let queued_message = NostrQueuedMessage {
                    id: message_id.to_string(),
                    sender_id: sender.to_owned(),
                    recipient: Some(node_id.to_owned()),
                    payload: payload.clone(),
                };

                let identity = get_identity_public_data(
                    &node_id,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity))));
                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Err(Error::Network("Failed to send".to_string())));

                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message.clone()]))
                    .times(1);
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]))
                    .times(1);

                mock_queue
                    .expect_fail_retry()
                    .with(eq(message_id))
                    .returning(|_| {
                        Err(bcr_ebill_persistence::Error::InsertFailed(
                            "Failed to update retry status".to_string(),
                        ))
                    });
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok()); // Should still return Ok despite the internal error
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_succeed_retry_error() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let node_id = node_id_test_other();
                let message_id = "test_message_id";
                let sender = node_id_test();
                let payload = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );

                let queued_message = NostrQueuedMessage {
                    id: message_id.to_string(),
                    sender_id: sender.to_owned(),
                    recipient: Some(node_id.to_owned()),
                    payload: payload.clone(),
                };

                let identity = get_identity_public_data(
                    &node_id,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity))));

                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Ok(()));

                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message.clone()]))
                    .times(1);
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]))
                    .times(1);

                mock_queue
                    .expect_succeed_retry()
                    .with(eq(message_id))
                    .returning(|_| {
                        Err(bcr_ebill_persistence::Error::InsertFailed(
                            "Failed to update retry status".to_string(),
                        ))
                    });
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok()); // Should still return Ok despite the internal error
    }

    #[tokio::test]
    async fn test_send_retry_messages_with_no_messages() {
        init_test_cfg();

        let (service, _) = expect_service(|_, _, _, mock_queue, _, _, _, _| {
            mock_queue
                .expect_get_retry_messages()
                .returning(|_| Ok(vec![]))
                .times(1);
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_failed_to_send_is_added_to_retry_queue() {
        init_test_cfg();
        // given a payer and payee with a new bill
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let (service, _) = expect_service(
            |mock,
             mock_contact_store,
             _,
             queue_mock,
             _,
             notification_transport,
             _,
             block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();

                // sending the block events succeeds
                block_transport
                    .expect_send_bill_chain_events()
                    .returning(|_| Ok(()))
                    .once();

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payer))));

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payee))));

                // one dm succeeds
                mock.expect_send_private_event()
                    .returning(|_, _, _| Ok(()))
                    .once();

                // now a chain invite should be sent but fails
                mock.expect_send_private_event()
                    .withf(move |_, _, e| {
                        let r: bcr_ebill_core::protocol::Result<Event<ChainInvite>> =
                            e.clone().try_into();
                        r.is_err()
                    })
                    .returning(|_, _, _| Err(Error::Network("Failed to send".to_string())));

                queue_mock
                    .expect_add_message()
                    .returning(|_, _| Ok(()))
                    .once();

                notification_transport
                    .expect_send_email_notification()
                    .returning(|_, _, _| ())
                    .once();
            },
        );

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        service
            .send_bill_is_signed_event(&event)
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_failed_to_send_returns_error_when_retry_enqueue_fails() {
        init_test_cfg();
        let payer = get_identity_public_data(
            &node_id_test(),
            &Email::new("drawee@example.com").unwrap(),
            vec![],
        );
        let payee = get_identity_public_data(
            &node_id_test_other(),
            &Email::new("payee@example.com").unwrap(),
            vec![],
        );
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let (service, _) = expect_service(
            |mock,
             mock_contact_store,
             _,
             queue_mock,
             _,
             notification_transport,
             _,
             block_transport| {
                let payer = payer.clone();
                let payee = payee.clone();

                block_transport
                    .expect_send_bill_chain_events()
                    .returning(|_| Ok(()))
                    .once();

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payer))));

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&payee))));

                mock.expect_send_private_event()
                    .returning(|_, _, _| Ok(()))
                    .once();

                mock.expect_send_private_event()
                    .withf(move |_, _, e| {
                        let r: bcr_ebill_core::protocol::Result<Event<ChainInvite>> =
                            e.clone().try_into();
                        r.is_err()
                    })
                    .returning(|_, _, _| Err(Error::Network("Failed to send".to_string())));

                queue_mock
                    .expect_add_message()
                    .returning(|_, _| {
                        Err(bcr_ebill_persistence::Error::Persistence(
                            "queue down".to_string(),
                        ))
                    })
                    .once();

                notification_transport
                    .expect_send_email_notification()
                    .never();
            },
        );

        let event = BillChainEvent::new(
            &bill,
            &chain,
            &BcrKeys::from_private_key(&private_key_test()),
            true,
            &node_id_test(),
        )
        .unwrap();

        let result = service.send_bill_is_signed_event(&event).await;
        assert!(
            matches!(result, Err(Error::Persistence(msg)) if msg.contains("Failed to add send nostr event to retry queue"))
        );
    }

    #[tokio::test]
    async fn test_send_retry_public_message_success() {
        init_test_cfg();

        let (service, _) = expect_service(|mock_transport, _, _, mock_queue, _, _, _, _| {
            let message_id = "test_public_message_id";
            let sender = node_id_test();
            let nostr_event = get_test_nostr_event();
            let payload = serde_json::to_string(&nostr_event).unwrap();

            let queued_message = NostrQueuedMessage {
                id: message_id.to_string(),
                sender_id: sender.to_owned(),
                recipient: None,
                payload,
            };

            mock_transport
                .expect_broadcast_event()
                .returning(|_| Ok(()));

            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(move |_| Ok(vec![queued_message.clone()]))
                .once();
            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(|_| Ok(vec![]));
            mock_queue
                .expect_succeed_retry()
                .with(eq(message_id))
                .returning(|_| Ok(()));
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_public_message_invalid_payload() {
        init_test_cfg();

        let (service, _) = expect_service(|_, _, _, mock_queue, _, _, _, _| {
            let message_id = "test_public_message_id";
            let sender = node_id_test();
            // Invalid JSON that can't be deserialized to nostr::event::Event
            let invalid_payload = "not valid json at all".to_string();

            let queued_message = NostrQueuedMessage {
                id: message_id.to_string(),
                sender_id: sender.to_owned(),
                recipient: None,
                payload: invalid_payload,
            };

            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(move |_| Ok(vec![queued_message.clone()]))
                .times(1);
            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(|_| Ok(vec![]))
                .times(1);
            mock_queue
                .expect_fail_retry()
                .with(eq(message_id.to_string()))
                .returning(|_| Ok(()))
                .times(1);
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_public_message_broadcast_failure() {
        init_test_cfg();

        let (service, _) = expect_service(|mock_transport, _, _, mock_queue, _, _, _, _| {
            let message_id = "test_public_message_id";
            let sender = node_id_test();
            let nostr_event = get_test_nostr_event();
            let payload = serde_json::to_string(&nostr_event).unwrap();

            let queued_message = NostrQueuedMessage {
                id: message_id.to_string(),
                sender_id: sender.to_owned(),
                recipient: None,
                payload,
            };

            mock_transport
                .expect_broadcast_event()
                .returning(|_| Err(Error::Network("relay unavailable".to_string())));

            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(move |_| Ok(vec![queued_message.clone()]))
                .once();
            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(|_| Ok(vec![]));
            mock_queue
                .expect_fail_retry()
                .with(eq(message_id.to_string()))
                .returning(|_| Ok(()));
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_messages_mixed_private_and_public() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                // Private message setup
                let private_node_id = node_id_test_other();
                let private_message_id = "private_msg_id";
                let sender = node_id_test();
                let private_payload = base58::encode(
                    &borsh::to_vec(&EventEnvelope {
                        version: "1.0".to_string(),
                        event_type: EventType::Bill,
                        data: vec![],
                    })
                    .unwrap(),
                );
                let private_message = NostrQueuedMessage {
                    id: private_message_id.to_string(),
                    sender_id: sender.to_owned(),
                    recipient: Some(private_node_id.to_owned()),
                    payload: private_payload,
                };

                // Public message setup
                let public_message_id = "public_msg_id";
                let nostr_event = get_test_nostr_event();
                let public_payload = serde_json::to_string(&nostr_event).unwrap();
                let public_message = NostrQueuedMessage {
                    id: public_message_id.to_string(),
                    sender_id: sender.to_owned(),
                    recipient: None,
                    payload: public_payload,
                };

                let identity = get_identity_public_data(
                    &private_node_id,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .returning(move |_| Ok(Some(as_contact(&identity))));

                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Ok(()));

                mock_transport
                    .expect_broadcast_event()
                    .returning(|_| Ok(()));

                // First call returns private message
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![private_message.clone()]))
                    .once();
                // Second call returns public message
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![public_message.clone()]))
                    .once();
                // Third call returns empty (done)
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]));

                mock_queue
                    .expect_succeed_retry()
                    .with(eq(private_message_id))
                    .returning(|_| Ok(()))
                    .once();
                mock_queue
                    .expect_succeed_retry()
                    .with(eq(public_message_id))
                    .returning(|_| Ok(()))
                    .once();
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_private_message_with_legacy_payload_format() {
        init_test_cfg();

        let (service, _) = expect_service(
            |mock_transport, mock_contact_store, _, mock_queue, _, _, _, _| {
                let recipient = node_id_test_other();
                let message_id = "private_legacy_msg_id";
                let sender = node_id_test();

                let legacy_event = Event::new_bill(BillChainEventPayload {
                    event_type: BillEventType::BillMintingRequested,
                    bill_id: bill_id_test(),
                    action_type: Some(ActionType::CheckBill),
                    sum: Some(Sum::new_sat(1).unwrap()),
                    sender_node_id: None,
                    sender_name: None,
                });

                let payload = base58::encode(&borsh::to_vec(&legacy_event).unwrap());

                let queued_message = NostrQueuedMessage {
                    id: message_id.to_string(),
                    sender_id: sender.to_owned(),
                    recipient: Some(recipient.to_owned()),
                    payload,
                };

                let identity = get_identity_public_data(
                    &recipient,
                    &Email::new("test@example.com").unwrap(),
                    vec![],
                );

                mock_contact_store
                    .expect_get()
                    .with(eq(recipient.to_owned()))
                    .returning(move |_| Ok(Some(as_contact(&identity))))
                    .once();

                mock_transport
                    .expect_send_private_event()
                    .returning(|_, _, _| Ok(()))
                    .once();

                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(move |_| Ok(vec![queued_message.clone()]))
                    .once();
                mock_queue
                    .expect_get_retry_messages()
                    .with(eq(1))
                    .returning(|_| Ok(vec![]))
                    .once();

                mock_queue
                    .expect_succeed_retry()
                    .with(eq(message_id))
                    .returning(|_| Ok(()))
                    .once();
            },
        );

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_retry_private_message_with_invalid_base58() {
        init_test_cfg();

        let (service, _) = expect_service(|_, _, _, mock_queue, _, _, _, _| {
            let node_id = node_id_test_other();
            let message_id = "test_bad_base58_id";
            let sender = node_id_test();
            let invalid_base58_payload = "0OIl!!!not_base58".to_string();

            let queued_message = NostrQueuedMessage {
                id: message_id.to_string(),
                sender_id: sender.to_owned(),
                recipient: Some(node_id.to_owned()),
                payload: invalid_base58_payload,
            };

            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(move |_| Ok(vec![queued_message.clone()]))
                .once();
            mock_queue
                .expect_get_retry_messages()
                .with(eq(1))
                .returning(|_| Ok(vec![]))
                .once();
            mock_queue
                .expect_fail_retry()
                .with(eq(message_id.to_string()))
                .returning(|_| Ok(()))
                .once();
        });

        let result = service.send_retry_messages().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_company_historical_bill_invites_success() {
        init_test_cfg();
        let company_id = node_id_test();
        let sender_npub = nostr::key::PublicKey::from_hex(
            "22886f449bec154764401cfb139b80f108a39a91c7e7609f9ffd8a4592b86d38",
        )
        .unwrap();

        let mut mock_transport = MockNotificationJsonTransport::new();
        mock_transport
            .expect_has_local_signer()
            .with(eq(company_id.clone()))
            .returning(|_| true);

        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&nostr::key::Keys::generate())
            .expect(" Could not create test event");

        let invite = ChainInvite::bill(bill_id_test().to_string(), BcrKeys::new());
        let envelope: EventEnvelope = Event::new(EventType::BillChainInvite, invite.clone())
            .try_into()
            .unwrap();

        let company_id_for_decrypt = company_id.clone();
        mock_transport
            .expect_resolve_events()
            .returning(move |_| Ok(vec![event.clone()]));
        mock_transport
            .expect_try_decrypt_private_event()
            .returning(move |_| {
                Ok(Some((
                    company_id_for_decrypt.clone(),
                    envelope.clone(),
                    sender_npub,
                )))
            });

        let mut handler = MockNotificationHandler::new();
        let company_id_for_handler = company_id.clone();
        handler
            .expect_handle_event()
            .withf(move |env, node_id, sender, _original_event| {
                env.event_type == EventType::BillChainInvite
                    && *node_id == company_id_for_handler
                    && sender.is_some()
            })
            .returning(|_, _, _, _| Ok(()))
            .times(1);

        let service = get_transport_with_handler(
            mock_transport,
            MockContactStore::new(),
            MockNostrContactStore::new(),
            MockNostrQueuedMessageStore::new(),
            MockNostrChainEventStore::new(),
            MockNotificationTransportService::new(),
            MockContactTransportService::new(),
            MockBlockTransportService::new(),
            Arc::new(handler),
        );

        let result = service
            .process_company_historical_bill_invites(&company_id)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_company_historical_bill_invites_skips_non_invite() {
        init_test_cfg();
        let company_id = node_id_test();
        let sender_npub = nostr::key::PublicKey::from_hex(
            "22886f449bec154764401cfb139b80f108a39a91c7e7609f9ffd8a4592b86d38",
        )
        .unwrap();

        let mut mock_transport = MockNotificationJsonTransport::new();
        mock_transport
            .expect_has_local_signer()
            .with(eq(company_id.clone()))
            .returning(|_| true);

        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&nostr::key::Keys::generate())
            .expect(" Could not create test event");

        let non_invite = ChainInvite::company(node_id_test().to_string(), BcrKeys::new());
        let envelope: EventEnvelope = Event::new(EventType::CompanyChainInvite, non_invite.clone())
            .try_into()
            .unwrap();

        let company_id_for_decrypt = company_id.clone();
        mock_transport
            .expect_resolve_events()
            .returning(move |_| Ok(vec![event.clone()]));
        mock_transport
            .expect_try_decrypt_private_event()
            .returning(move |_| {
                Ok(Some((
                    company_id_for_decrypt.clone(),
                    envelope.clone(),
                    sender_npub,
                )))
            });

        let mut handler = MockNotificationHandler::new();
        handler.expect_handle_event().times(0);

        let service = get_transport_with_handler(
            mock_transport,
            MockContactStore::new(),
            MockNostrContactStore::new(),
            MockNostrQueuedMessageStore::new(),
            MockNostrChainEventStore::new(),
            MockNotificationTransportService::new(),
            MockContactTransportService::new(),
            MockBlockTransportService::new(),
            Arc::new(handler),
        );

        let result = service
            .process_company_historical_bill_invites(&company_id)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_company_historical_bill_invites_no_signer() {
        init_test_cfg();
        let company_id = node_id_test();

        let mut mock_transport = MockNotificationJsonTransport::new();
        mock_transport
            .expect_has_local_signer()
            .with(eq(company_id.clone()))
            .returning(|_| false);
        mock_transport.expect_resolve_events().times(0);

        let service = get_transport_with_handler(
            mock_transport,
            MockContactStore::new(),
            MockNostrContactStore::new(),
            MockNostrQueuedMessageStore::new(),
            MockNostrChainEventStore::new(),
            MockNotificationTransportService::new(),
            MockContactTransportService::new(),
            MockBlockTransportService::new(),
            Arc::new(MockNotificationHandler::new()),
        );

        let result = service
            .process_company_historical_bill_invites(&company_id)
            .await;
        assert!(result.is_ok());
    }
}
