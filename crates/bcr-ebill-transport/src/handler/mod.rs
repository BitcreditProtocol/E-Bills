use crate::{Result, handler::public_chain_helpers::EventContainer};
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::blockchain::{bill::BillBlock, company::CompanyBlock, identity::IdentityBlock},
    protocol::event::EventEnvelope,
};
use log::trace;
#[cfg(test)]
use mockall::automock;

use super::EventType;

mod bill_action_event_handler;
mod bill_chain_event_handler;
mod bill_chain_event_processor;
mod bill_invite_handler;
mod company_chain_event_handler;
mod company_chain_event_processor;
mod company_invite_handler;
mod contact_share_handler;
mod direct_message_event_processor;
mod file_metadata_processor;
mod identity_chain_event_handler;
mod identity_chain_event_processor;
mod inbound_file_anchor;
mod nostr_contact_processor;
pub(crate) mod public_chain_helpers;
mod restore;

pub use bill_action_event_handler::BillActionEventHandler;
pub use bill_chain_event_handler::BillChainEventHandler;
pub use bill_chain_event_processor::BillChainEventProcessor;
pub use bill_invite_handler::BillInviteEventHandler;
pub use company_chain_event_handler::CompanyChainEventHandler;
pub use company_chain_event_processor::CompanyChainEventProcessor;
pub use company_invite_handler::CompanyInviteEventHandler;
pub use contact_share_handler::ContactShareEventHandler;
pub use direct_message_event_processor::DirectMessageEventProcessor;
pub use file_metadata_processor::{FileMetadataProcessor, FileMetadataProcessorApi};
pub use identity_chain_event_handler::IdentityChainEventHandler;
pub use identity_chain_event_processor::IdentityChainEventProcessor;
pub use nostr_contact_processor::NostrContactProcessor;
pub use restore::RestoreAccountService;

#[cfg(test)]
impl ServiceTraitBounds for MockNotificationHandlerApi {}

/// Handle an event when we receive it from a channel.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait NotificationHandlerApi: ServiceTraitBounds {
    /// Whether this handler handles the given event type.
    fn handles_event(&self, event_type: &EventType) -> bool;

    /// Handle the event. This is called by the notification processor which should
    /// have checked the event type before calling this method. The actual implementation
    /// should be able to deserialize the data into its T type because the EventType
    /// determines the T type. Identity represents the active identity that is receiving
    /// the event.
    async fn handle_event(
        &self,
        event: bcr_ebill_core::protocol::event::EventEnvelope,
        node_id: &NodeId,
        sender: Option<nostr::key::PublicKey>,
        original_event: Option<Box<nostr::event::Event>>,
    ) -> Result<()>;
}

/// Generalizes the actual handling and validation of a bill block event.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait BillChainEventProcessorApi: ServiceTraitBounds {
    /// Processes the chain data for given bill id, some blocks and an optional key that will be
    /// present when we are joining a new chain.
    async fn process_chain_data(
        &self,
        bill_id: &BillId,
        blocks: Vec<BillBlock>,
        keys: Option<BcrKeys>,
    ) -> Result<()>;

    /// Validates that a given bill id is relevant for us, and if so also checks that the sender
    /// of the event is part of the chain this event is for.
    async fn validate_chain_event_and_sender(
        &self,
        bill_id: &BillId,
        sender: nostr::key::PublicKey,
    ) -> Result<bool>;

    /// Resolves the Bill chain blocks from Nostr for the given bill id.
    async fn resolve_chain(
        &self,
        bill_id: &BillId,
        bill_keys: &BcrKeys,
    ) -> Result<Vec<Vec<EventContainer>>>;

    /// Tries to resync the chain for the given bill id. If `from_nostr` is true, this will try to
    /// find the bill keys and then try to find the chain data from Nostr. Will add all potentially
    /// missing blocks to the chain. If `from_nostr` is false, only invalidates the local cache.
    async fn resync_chain(&self, bill_id: &BillId, from_nostr: bool) -> Result<()>;

    /// Invalidates the cached bill data for the given bill id.
    async fn invalidate_cache_for_bill(&self, bill_id: &BillId) -> Result<()>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockBillChainEventProcessorApi {}

/// Generalizes the handling and validation of a bill block event.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait CompanyChainEventProcessorApi: ServiceTraitBounds {
    /// Processes the chain data for given bill id, some blocks and an optional key that will be
    /// present when we are joining a new chain.
    async fn process_chain_data(
        &self,
        node_id: &NodeId,
        blocks: Vec<CompanyBlock>,
        keys: Option<BcrKeys>,
    ) -> Result<()>;

    /// Validates that a given bill id is relevant for us, and if so also checks that the sender
    /// of the event is part of the chain this event is for.
    async fn validate_chain_event_and_sender(
        &self,
        node_id: &NodeId,
        sender: nostr::key::PublicKey,
    ) -> Result<bool>;

    /// Tries to resync the chain for the given node id. This will try to find the company keys and
    /// then try to find the chain data for the given company id. Will add all potentially missing
    /// blocks to the chain.
    async fn resync_chain(&self, company_id: &NodeId) -> Result<()>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockCompanyChainEventProcessorApi {}

/// Generalizes the handling and validation of a bill block event.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait IdentityChainEventProcessorApi: ServiceTraitBounds {
    /// Processes the chain data for given bill id, some blocks and an optional key that will be
    /// present when we are joining a new chain.
    async fn process_chain_data(
        &self,
        node_id: &NodeId,
        blocks: Vec<IdentityBlock>,
        keys: Option<BcrKeys>,
    ) -> Result<()>;

    /// Validates that a given bill id is relevant for us, and if so also checks that the sender
    /// of the event is part of the chain this event is for.
    fn validate_chain_event_and_sender(
        &self,
        node_id: &NodeId,
        sender: nostr::key::PublicKey,
    ) -> bool;

    /// Tries to resync the chain for the primary local identity. This will try to find the chain data for the current identity.
    /// Will add all potentially missing blocks to the chain.
    async fn resync_chain(&self) -> Result<()>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockIdentityChainEventProcessorApi {}

/// Generalizes the handling and validation of direct messages.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait DirectMessageEventProcessorApi: ServiceTraitBounds {
    async fn process_direct_message(&self, event: Box<nostr::event::Event>) -> Result<()>;
}

#[cfg(test)]
impl ServiceTraitBounds for MockDirectMessageEventProcessorApi {}

/// Generalizes the handling of other Nostr identities.
#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait NostrContactProcessorApi: ServiceTraitBounds {
    /// Ensures that a given node id is in our Nostr contacts. If not it will be added
    /// with data fetched from Nostr relays.
    async fn ensure_nostr_contact(&self, node_id: &NodeId);
}

#[cfg(test)]
impl ServiceTraitBounds for MockNostrContactProcessorApi {}

/// Logs all events that are received and registered in the event_types.
pub struct LoggingEventHandler {
    pub event_types: Vec<EventType>,
}

impl ServiceTraitBounds for LoggingEventHandler {}

/// Just a dummy handler that logs the event and returns Ok(())
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationHandlerApi for LoggingEventHandler {
    fn handles_event(&self, event_type: &EventType) -> bool {
        self.event_types.contains(event_type)
    }

    async fn handle_event(
        &self,
        event: EventEnvelope,
        identity: &NodeId,
        _: Option<nostr::key::PublicKey>,
        _: Option<Box<nostr::event::Event>>,
    ) -> Result<()> {
        trace!("Received event: {event:?} for identity: {identity}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bcr_ebill_core::protocol::event::BillEventType;
    use bcr_ebill_core::protocol::event::Event;
    use borsh::{BorshDeserialize, BorshSerialize};
    use std::str::FromStr;
    use tokio::sync::Mutex;

    use crate::handler::test_utils::get_test_nostr_event;

    use super::*;

    #[tokio::test]
    async fn test_event_handling() {
        let accepted_event = EventType::Bill;

        // given a handler that accepts the event type
        let event_handler: TestEventHandler<TestEventPayload> =
            TestEventHandler::new(Some(accepted_event.to_owned()));

        // event type should be accepted
        assert!(event_handler.handles_event(&accepted_event));

        // given an event and encode it to an envelope
        let event = create_test_event(&BillEventType::BillPaid);
        let envelope: EventEnvelope = event.clone().try_into().unwrap();
        let nostr_event = Box::new(get_test_nostr_event());

        // handler should run successfully
        event_handler
            .handle_event(
                envelope,
                &NodeId::from_str(
                    "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
                )
                .unwrap(),
                None,
                Some(nostr_event),
            )
            .await
            .expect("event was not handled");

        // handler should have been invoked
        let called = event_handler.called.lock().await;
        assert!(*called, "event was not handled");

        // and the event should have been received
        let received = event_handler.received_event.lock().await.clone().unwrap();
        assert_eq!(event.data, received.data, "handled payload was not correct");
    }

    #[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
    struct TestEventPayload {
        pub event_type: BillEventType,
        pub foo: String,
        pub bar: u32,
    }

    struct TestEventHandler<T: BorshSerialize + BorshDeserialize> {
        pub called: Mutex<bool>,
        pub received_event: Mutex<Option<Event<T>>>,
        pub accepted_event: Option<EventType>,
    }

    impl<T: BorshSerialize + BorshDeserialize + Send + Sync> ServiceTraitBounds
        for TestEventHandler<T>
    {
    }

    impl<T: BorshSerialize + BorshDeserialize> TestEventHandler<T> {
        pub fn new(accepted_event: Option<EventType>) -> Self {
            Self {
                called: Mutex::new(false),
                received_event: Mutex::new(None),
                accepted_event,
            }
        }
    }

    #[async_trait]
    impl NotificationHandlerApi for TestEventHandler<TestEventPayload> {
        fn handles_event(&self, event_type: &EventType) -> bool {
            match &self.accepted_event {
                Some(e) => e == event_type,
                None => true,
            }
        }

        async fn handle_event(
            &self,
            event: EventEnvelope,
            _: &NodeId,
            _: Option<nostr::key::PublicKey>,
            _: Option<Box<nostr::event::Event>>,
        ) -> Result<()> {
            *self.called.lock().await = true;
            let event: Event<TestEventPayload> = event.try_into()?;
            *self.received_event.lock().await = Some(event);
            Ok(())
        }
    }

    fn create_test_event_payload(event_type: &BillEventType) -> TestEventPayload {
        TestEventPayload {
            event_type: event_type.clone(),
            foo: "foo".to_string(),
            bar: 42,
        }
    }

    fn create_test_event(event_type: &BillEventType) -> Event<TestEventPayload> {
        Event::new(EventType::Bill, create_test_event_payload(event_type))
    }
}

#[allow(dead_code)]
#[cfg(test)]
mod test_utils {
    use async_trait::async_trait;
    use bcr_common::cashu::{self};
    use bcr_common::core::{BillId, NodeId};
    use bcr_ebill_api::external::mint::MintClientApi;
    use bcr_ebill_api::external::mint::{QuoteStatusReply, ResolveMintOffer};
    use bcr_ebill_core::application::company::{
        CompanySignatory, CompanySignatoryStatus, CompanyStatus,
    };
    use bcr_ebill_core::application::nostr_contact::{
        HandshakeStatus, NostrContact, NostrPublicKey, TrustLevel,
    };
    use bcr_ebill_core::application::{
        ServiceTraitBounds,
        bill::{BitcreditBillResult, PaymentState},
        company::{Company, LocalSignatoryOverride, LocalSignatoryOverrideStatus},
        identity::{Identity, IdentityWithAll},
        notification::{Notification, NotificationType},
    };
    use bcr_ebill_core::protocol::blockchain::company::block::{
        CompanyUpdateBlockData, SignatoryType,
    };
    use bcr_ebill_core::protocol::blockchain::identity::IdentityUpdateBlockData;
    use bcr_ebill_core::protocol::{
        Address, BlockId, City, Country, Date, Email, Identification, Name, OptionalPostalAddress,
        PostalAddress, PublicKey, SecretKey, Sum, Timestamp,
        blockchain::bill::participant::{BillIdentParticipant, BillParticipant},
        blockchain::{
            bill::{
                BillBlock, BillBlockchain, BillOpCode, BitcreditBill,
                block::{BillIssueBlockData, ContactType},
            },
            company::{CompanyBlock, CompanyBlockchain},
            identity::IdentityType,
        },
        crypto::BcrKeys,
        event::ActionType,
    };
    use bcr_ebill_core::protocol::{
        BitcoinAddress, EditOptionalFieldMode, EmailIdentityProofData, Sha256Hash,
        SignedIdentityProof, blockchain::bill::BillToShareWithExternalParty,
    };
    use bcr_ebill_persistence::{
        NostrChainEventStoreApi, NotificationStoreApi, Result, ShareDirection,
        bill::{BillChainStoreApi, BillStoreApi},
        company::{CompanyChainStoreApi, CompanyStoreApi},
        identity::{IdentityChainStoreApi, IdentityStoreApi},
        nostr::{NostrContactStoreApi, PendingContactShare, RelaySyncStatus, SyncStatus},
        notification::NotificationFilter,
    };
    use mockall::mock;
    use nostr::event::{EventBuilder, FinalizeEvent};
    use std::{collections::HashMap, str::FromStr};

    use crate::PushApi;
    use crate::test_utils::{signed_identity_proof_test, test_ts};

    mock! {
        pub MintClient {}

        impl ServiceTraitBounds for MintClient {}

        #[async_trait]
        impl MintClientApi for MintClient {
            async fn check_if_proofs_are_spent(
                &self,
                mint_url: &url::Url,
                proofs: &str,
                keyset_id: &str,
            ) -> bcr_ebill_api::external::mint::Result<bool>;
            async fn mint(
                &self,
                bill_id: &BillId,
                mint_url: &url::Url,
                keyset: bcr_common::ecash::KeySet,
                quote_id: &uuid::Uuid,
                private_key: &SecretKey,
                blinded_messages: Vec<cashu::BlindedMessage>,
                secrets: Vec<cashu::secret::Secret>,
                rs: Vec<cashu::SecretKey>,
            ) -> bcr_ebill_api::external::mint::Result<String>;
            async fn get_keyset_info(&self, mint_url: &url::Url, keyset_id: &str) -> bcr_ebill_api::external::mint::Result<bcr_common::ecash::KeySet>;
            async fn enquire_mint_quote(
                &self,
                mint_url: &url::Url,
                bill_to_share: BillToShareWithExternalParty,
                requester_keys: &BcrKeys,
            ) -> bcr_ebill_api::external::mint::Result<uuid::Uuid>;
            async fn lookup_quote_for_mint(
                &self,
                mint_url: &url::Url,
                quote_id: &uuid::Uuid,
            ) -> bcr_ebill_api::external::mint::Result<QuoteStatusReply>;
            async fn resolve_quote_for_mint(
                &self,
                mint_url: &url::Url,
                quote_id: &uuid::Uuid,
                resolve: ResolveMintOffer,
            ) -> bcr_ebill_api::external::mint::Result<()>;
            async fn cancel_quote_for_mint(&self, mint_url: &url::Url, quote_id: &uuid::Uuid) -> bcr_ebill_api::external::mint::Result<()>;
            async fn validate_payment_address_from_mint(
                &self,
                mint_url: &url::Url,
                address_to_validate: &BitcoinAddress,
                bill_id: &BillId,
                block_id: BlockId,
                previous_block_hash: &Sha256Hash,
            ) -> bcr_ebill_api::external::mint::Result<()>;
        }
    }

    mock! {
        pub NotificationStore {}

        impl ServiceTraitBounds for NotificationStore {}

        #[async_trait]
        impl NotificationStoreApi for NotificationStore {
            async fn get_active_status_for_node_ids(
                &self,
                node_ids: &[NodeId],
            ) -> Result<HashMap<NodeId, bool>>;
            async fn add(&self, notification: Notification) -> Result<Notification>;
            async fn list(&self, filter: NotificationFilter) -> Result<Vec<Notification>>;
            async fn get_latest_by_references(
                &self,
                reference: &[String],
                notification_type: NotificationType,
            ) -> Result<HashMap<String, Notification>>;
            async fn get_latest_by_reference(
                &self,
                reference: &str,
                notification_type: NotificationType,
            ) -> Result<Option<Notification>>;
            async fn get_latest_by_reference_and_node_id(
                &self,
                reference: &str,
                notification_type: NotificationType,
                node_id: &NodeId,
            ) -> Result<Option<Notification>>;
            #[allow(unused)]
            async fn list_by_type(&self, notification_type: bcr_ebill_core::application::notification::NotificationType) -> Result<Vec<Notification>>;
            async fn mark_as_done(&self, notification_id: &str) -> Result<()>;
            #[allow(unused)]
            async fn delete(&self, notification_id: &str) -> Result<()>;
            async fn set_bill_notification_sent(
                &self,
                bill_id: &BillId,
                block_height: i32,
                action_type: ActionType,
            ) -> Result<()>;
            async fn bill_notification_sent(
                &self,
                bill_id: &BillId,
                block_height: i32,
                action_type: ActionType,
            ) -> Result<bool>;
            async fn notification_exists_for_event_id(
                &self,
                event_id: &str,
                node_id: &NodeId,
            ) -> Result<bool>;
        }
    }

    mock! {
        pub PushService {}

        impl ServiceTraitBounds for PushService {}

        #[async_trait]
        impl PushApi for PushService {
            async fn send(&self, value: serde_json::Value);
            async fn subscribe(&self) -> async_broadcast::Receiver<serde_json::Value> ;
        }
    }

    mock! {
        pub BillChainStore {}

        impl ServiceTraitBounds for BillChainStore {}

        #[async_trait]
        impl BillChainStoreApi for BillChainStore {
            async fn get_latest_block(&self, id: &BillId) -> Result<BillBlock>;
            async fn add_block(&self, id: &BillId, block: &BillBlock) -> Result<()>;
            async fn get_chain(&self, id: &BillId) -> Result<BillBlockchain>;
            async fn remove_blocks_from_height(&self, id: &BillId, from_block_id: BlockId) -> Result<()>;
        }
    }

    mock! {
        pub BillStore {}

        impl ServiceTraitBounds for BillStore {}

        #[async_trait]
        impl BillStoreApi for BillStore {
            async fn get_bills_from_cache(&self, ids: &[BillId], identity_node_id: &NodeId) -> Result<Vec<BitcreditBillResult>>;
            async fn get_bill_from_cache(&self, id: &BillId, identity_node_id: &NodeId) -> Result<Option<BitcreditBillResult>>;
            async fn save_bill_to_cache(&self, id: &BillId, identity_node_id: &NodeId, bill: &BitcreditBillResult) -> Result<()>;
            async fn invalidate_bill_in_cache(&self, id: &BillId) -> Result<()>;
            async fn clear_bill_cache(&self) -> Result<()>;
            async fn exists(&self, id: &BillId) -> Result<bool>;
            async fn get_ids(&self) -> Result<Vec<BillId>>;
            async fn save_keys(&self, id: &BillId, keys: &BcrKeys) -> Result<()>;
            async fn get_keys(&self, id: &BillId) -> Result<BcrKeys>;
            async fn is_paid(&self, id: &BillId) -> Result<bool>;
            async fn set_payment_state(&self, id: &BillId, payment_state: &PaymentState) -> Result<()>;
            async fn get_payment_state(&self, id: &BillId) -> Result<Option<PaymentState>>;
            async fn set_offer_to_sell_payment_state(
                &self,
                id: &BillId,
                block_id: BlockId,
                payment_state: &PaymentState,
            ) -> Result<()>;
            async fn get_offer_to_sell_payment_state(
                &self,
                id: &BillId,
                block_id: BlockId,
            ) -> Result<Option<PaymentState>>;
            async fn set_recourse_payment_state(
                &self,
                id: &BillId,
                block_id: BlockId,
                payment_state: &PaymentState,
            ) -> Result<()>;
            async fn get_recourse_payment_state(
                &self,
                id: &BillId,
                block_id: BlockId,
            ) -> Result<Option<PaymentState>>;
            async fn get_bill_ids_waiting_for_payment(&self) -> Result<Vec<BillId>>;
            async fn get_bill_ids_waiting_for_sell_payment(&self) -> Result<Vec<BillId>>;
            async fn get_bill_ids_waiting_for_recourse_payment(&self) -> Result<Vec<BillId>>;
            async fn get_bill_ids_with_op_codes_since(
                &self,
                op_code: std::collections::HashSet<BillOpCode> ,
                since: Timestamp,
            ) -> Result<Vec<BillId>>;
        }
    }

    mock! {
        pub NostrContactStore {}

        impl ServiceTraitBounds for NostrContactStore {}

        #[async_trait]
        impl NostrContactStoreApi for NostrContactStore {
            async fn by_node_id(&self, node_id: &NodeId) -> Result<Option<NostrContact>>;
            async fn by_node_ids(&self, node_ids: Vec<NodeId>) -> Result<Vec<NostrContact>>;
            async fn get_all(&self) -> Result<Vec<NostrContact>>;
            async fn by_npub(&self, npub: &NostrPublicKey) -> Result<Option<NostrContact>>;
            async fn upsert(&self, data: &NostrContact) -> Result<()>;
            async fn delete(&self, node_id: &NodeId) -> Result<()>;
            async fn set_handshake_status(&self, node_id: &NodeId, status: HandshakeStatus) -> Result<()>;
            async fn set_trust_level(&self, node_id: &NodeId, trust_level: TrustLevel) -> Result<()>;
            async fn get_npubs(&self, levels: Vec<TrustLevel>) -> Result<Vec<NostrPublicKey>>;
            async fn search(&self, search_term: &str, levels: Vec<TrustLevel>) -> Result<Vec<NostrContact>>;
            async fn add_pending_share(&self, pending_share: PendingContactShare) -> Result<()>;
            async fn get_pending_share(&self, id: &str) -> Result<Option<PendingContactShare>>;
            async fn get_pending_share_by_private_key(&self, private_key: &SecretKey) -> Result<Option<PendingContactShare>>;
            async fn list_pending_shares_by_receiver(&self, receiver_node_id: &NodeId) -> Result<Vec<PendingContactShare>>;
            async fn list_pending_shares_by_receiver_and_direction(&self, receiver_node_id: &NodeId, direction: ShareDirection) -> Result<Vec<PendingContactShare>>;
            async fn delete_pending_share(&self, id: &str) -> Result<()>;
            async fn pending_share_exists_for_node_and_receiver(&self, node_id: &NodeId, receiver_node_id: &NodeId, direction: ShareDirection) -> Result<bool>;
            async fn get_pending_relays(&self) -> Result<Vec<url::Url>>;
            async fn get_relay_sync_status(&self, relay: &url::Url) -> Result<Option<RelaySyncStatus>>;
            async fn update_relay_sync_status(&self, relay: &url::Url, status: SyncStatus) -> Result<()>;
            async fn update_relay_sync_progress(&self, relay: &url::Url, timestamp: bcr_ebill_core::protocol::Timestamp) -> Result<()>;
            async fn update_relay_last_seen(&self, relay: &url::Url, timestamp: bcr_ebill_core::protocol::Timestamp) -> Result<()>;
            async fn add_failed_relay_sync(&self, relay: &url::Url, event: nostr::event::Event) -> Result<()>;
            async fn get_pending_relay_retries(&self, relay: &url::Url, limit: usize) -> Result<Vec<nostr::event::Event>>;
            async fn mark_relay_retry_success(&self, relay: &url::Url, event_id: &str) -> Result<()>;
            async fn mark_relay_retry_failed(&self, relay: &url::Url, event_id: &str, max_retries: usize) -> Result<()>;
        }
    }

    mock! {
        pub IdentityStore {}

        impl ServiceTraitBounds for IdentityStore {}

        #[async_trait]
        impl IdentityStoreApi for IdentityStore {
            async fn exists(&self) -> bool;
            async fn save(&self, identity: &Identity) -> Result<()>;
            async fn get(&self) -> Result<Identity>;
            async fn get_full(&self) -> Result<IdentityWithAll>;
            async fn save_key_pair(&self, key_pair: &BcrKeys, seed: &str) -> Result<()>;
            async fn get_key_pair(&self) -> Result<BcrKeys>;
            async fn get_or_create_key_pair(&self) -> Result<BcrKeys>;
            async fn get_seedphrase(&self) -> Result<String>;
            async fn get_current_identity(&self) -> Result<bcr_ebill_core::application::identity::ActiveIdentityState>;
            async fn set_current_identity(&self, identity_state: &bcr_ebill_core::application::identity::ActiveIdentityState) -> Result<()>;
            async fn set_or_check_network(&self, configured_network: bitcoin::Network) -> Result<()>;
            async fn get_email_confirmations(
                &self,
            ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>>;
            async fn set_email_confirmation(
                &self,
                proof: &SignedIdentityProof,
                data: &EmailIdentityProofData,
            ) -> Result<()>;
        }
    }

    mock! {
        pub IdentityChainStore {}

        impl ServiceTraitBounds for IdentityChainStore {}

        #[async_trait]
        impl IdentityChainStoreApi for IdentityChainStore {
            async fn get_latest_block(&self) -> Result<bcr_ebill_core::protocol::blockchain::identity::IdentityBlock>;
            async fn add_block(&self, block: &bcr_ebill_core::protocol::blockchain::identity::IdentityBlock) -> Result<()>;
            async fn get_chain(&self) -> Result<bcr_ebill_core::protocol::blockchain::identity::IdentityBlockchain>;
            async fn remove_blocks_from_height(&self, from_block_id: BlockId) -> Result<()>;
        }
    }

    mock! {
        pub CompanyStore {}

        impl ServiceTraitBounds for CompanyStore {}

        #[async_trait]
        impl CompanyStoreApi for CompanyStore {
            async fn search(&self, search_term: &str) -> Result<Vec<Company>>;
            async fn exists(&self, id: &NodeId) -> bool;
            async fn get(&self, id: &NodeId) -> Result<Company>;
            async fn get_all(&self) -> Result<HashMap<NodeId, (Company, BcrKeys)>>;
            async fn insert(&self, data: &Company) -> Result<()>;
            async fn update(&self, id: &NodeId, data: &Company) -> Result<()>;
            async fn remove(&self, id: &NodeId) -> Result<()>;
            async fn save_key_pair(&self, id: &NodeId, key_pair: &BcrKeys) -> Result<()>;
            async fn get_key_pair(&self, id: &NodeId) -> Result<BcrKeys>;
            async fn get_email_confirmations(
                &self,
                id: &NodeId,
            ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>>;
            async fn set_email_confirmation(
                &self,
                id: &NodeId,
                proof: &SignedIdentityProof,
                data: &EmailIdentityProofData,
            ) -> Result<()>;
            async fn get_local_signatory_overrides(
                &self,
                id: &NodeId,
            ) -> Result<Vec<LocalSignatoryOverride>>;
            async fn set_local_signatory_override(
                &self,
                id: &NodeId,
                signatory: &NodeId,
                status: LocalSignatoryOverrideStatus,
            ) -> Result<()>;
            async fn delete_local_signatory_override(&self, id: &NodeId, signatory: &NodeId) -> Result<()>;
            async fn get_active_company_invites(&self) -> Result<HashMap<NodeId, (Company, BcrKeys)>>;
        }
    }

    mock! {
        pub CompanyChainStore {}

        impl ServiceTraitBounds for CompanyChainStore {}

        #[async_trait]
        impl CompanyChainStoreApi for CompanyChainStore {
            async fn get_latest_block(&self, id: &NodeId) -> Result<CompanyBlock>;
            async fn add_block(&self, id: &NodeId, block: &CompanyBlock) -> Result<()>;
            async fn remove(&self, id: &NodeId) -> Result<()>;
            async fn get_chain(&self, id: &NodeId) -> Result<CompanyBlockchain>;
            async fn remove_blocks_from_height(&self, id: &NodeId, from_block_id: BlockId) -> Result<()>;
        }
    }

    mock! {
        pub NostrChainEventStore {}
        impl ServiceTraitBounds for NostrChainEventStore {}

        #[async_trait]
        impl NostrChainEventStoreApi for NostrChainEventStore {
          async fn find_chain_events(
              &self,
              chain_id: &str,
              chain_type: bcr_ebill_core::protocol::blockchain::BlockchainType,
          ) -> Result<Vec<bcr_ebill_persistence::nostr::NostrChainEvent>>;
          async fn find_latest_block_events(
              &self,
              chain_id: &str,
              chain_type: bcr_ebill_core::protocol::blockchain::BlockchainType,
          ) -> Result<Vec<bcr_ebill_persistence::nostr::NostrChainEvent>>;
          async fn find_root_event(
              &self,
              chain_id: &str,
              chain_type: bcr_ebill_core::protocol::blockchain::BlockchainType,
          ) -> Result<Option<bcr_ebill_persistence::nostr::NostrChainEvent>>;
          async fn find_by_block_hash(&self, hash: &bcr_ebill_core::protocol::Sha256Hash) -> Result<Option<bcr_ebill_persistence::nostr::NostrChainEvent>>;
          async fn add_chain_event(&self, event: bcr_ebill_persistence::nostr::NostrChainEvent) -> Result<()>;
          async fn by_event_id(&self, event_id: &str) -> Result<Option<bcr_ebill_persistence::nostr::NostrChainEvent>>;
          async fn remove_chain_events(&self, chain_id: &str, chain_type: bcr_ebill_core::protocol::blockchain::BlockchainType) -> Result<()>;
        }
    }

    pub fn get_test_nostr_event() -> nostr::event::Event {
        EventBuilder::new(nostr::event::Kind::TextNote, "message")
            .finalize(&nostr::key::Keys::generate())
            .expect("Could not create nostr test event")
    }

    pub fn get_test_bitcredit_bill(
        id: &BillId,
        payer: &BillIdentParticipant,
        payee: &BillIdentParticipant,
        drawer: Option<&BillIdentParticipant>,
        endorsee: Option<&BillIdentParticipant>,
    ) -> BitcreditBill {
        let mut bill = empty_bitcredit_bill();
        bill.id = id.to_owned();
        bill.payee = BillParticipant::Ident(payee.clone());
        bill.drawee = payer.clone();
        if let Some(drawer) = drawer {
            bill.drawer = drawer.clone();
        }
        bill.endorsee = endorsee.map(|e| BillParticipant::Ident(e.to_owned()));
        bill
    }
    pub fn get_genesis_chain(bill: Option<BitcreditBill>) -> BillBlockchain {
        let bill = bill.unwrap_or(get_baseline_bill(&bill_id_test()));
        BillBlockchain::new(
            &BillIssueBlockData::from(bill, None, test_ts(), signed_identity_proof_test()),
            get_baseline_identity().key_pair,
            None,
            BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap()
    }
    pub fn get_baseline_bill(bill_id: &BillId) -> BitcreditBill {
        let mut bill = empty_bitcredit_bill();
        let keys = BcrKeys::new();

        bill.maturity_date = Date::new("2099-10-15").unwrap();
        let mut payee = empty_bill_identified_participant();
        payee.name = Name::new("payee").unwrap();
        payee.node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        bill.payee = BillParticipant::Ident(payee);
        bill.drawee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        bill.id = bill_id.to_owned();
        bill
    }
    pub fn empty_bitcredit_bill() -> BitcreditBill {
        BitcreditBill {
            id: bill_id_test(),
            country_of_issuing: Country::AT,
            city_of_issuing: City::new("Vienna").unwrap(),
            drawee: empty_bill_identified_participant(),
            drawer: empty_bill_identified_participant(),
            payee: BillParticipant::Ident(empty_bill_identified_participant()),
            endorsee: None,
            sum: Sum::new_sat(500).expect("sat works"),
            maturity_date: Date::new("2099-11-12").unwrap(),
            issue_date: Date::new("2099-08-12").unwrap(),
            city_of_payment: City::new("Vienna").unwrap(),
            country_of_payment: Country::AT,
            files: vec![],
        }
    }

    pub fn get_bill_keys() -> BcrKeys {
        BcrKeys::from_private_key(&private_key_test())
    }

    pub fn get_baseline_identity() -> IdentityWithAll {
        let keys = BcrKeys::from_private_key(&private_key_test());
        let mut identity = empty_identity();
        identity.name = Name::new("drawer").unwrap();
        identity.node_id = node_id_test();
        identity.postal_address.country = Some(Country::AT);
        identity.postal_address.city = Some(City::new("Vienna").unwrap());
        identity.postal_address.address = Some(Address::new("Hayekweg 5").unwrap());
        IdentityWithAll {
            identity,
            key_pair: keys,
        }
    }
    pub fn empty_bill_identified_participant() -> BillIdentParticipant {
        BillIdentParticipant {
            t: ContactType::Person,
            node_id: node_id_test(),
            name: Name::new("some name").unwrap(),
            postal_address: empty_address(),
            email: None,
            nostr_relays: vec![],
        }
    }
    pub fn empty_address() -> PostalAddress {
        PostalAddress {
            country: Country::AT,
            city: City::new("Vienna").unwrap(),
            zip: None,
            address: Address::new("Some address").unwrap(),
        }
    }
    pub fn empty_identity() -> Identity {
        Identity {
            t: IdentityType::Ident,
            node_id: node_id_test(),
            name: Name::new("some name").unwrap(),
            email: Some(Email::new("some@example.com").unwrap()),
            postal_address: empty_optional_address(),
            date_of_birth: None,
            country_of_birth: None,
            city_of_birth: None,
            identification_number: None,
            nostr_relays: vec![],
            profile_picture_file: None,
            identity_document_file: None,
        }
    }

    pub fn empty_optional_address() -> OptionalPostalAddress {
        OptionalPostalAddress {
            country: None,
            city: None,
            zip: None,
            address: None,
        }
    }

    pub fn get_company_data() -> (NodeId, (Company, BcrKeys)) {
        (
            node_id_test(),
            (
                Company {
                    id: node_id_test(),
                    name: Name::new("some_name").unwrap(),
                    country_of_registration: Some(Country::AT),
                    city_of_registration: Some(City::new("Vienna").unwrap()),
                    postal_address: empty_address(),
                    email: Email::new("company@example.com").unwrap(),
                    registration_number: Some(Identification::new("some_number").unwrap()),
                    registration_date: Some(Date::new("2012-01-01").unwrap()),
                    proof_of_registration_file: None,
                    logo_file: None,
                    signatories: vec![get_valid_activated_signatory(&node_id_test())],
                    creation_time: test_ts(),
                    status: CompanyStatus::Active,
                },
                BcrKeys::from_private_key(&private_key_test()),
            ),
        )
    }

    pub fn get_valid_activated_signatory(node_id: &NodeId) -> CompanySignatory {
        let (proof, data) = signed_identity_proof_test();
        CompanySignatory {
            t: SignatoryType::Solo,
            node_id: node_id.to_owned(),
            status: CompanySignatoryStatus::InviteAcceptedIdentityProven {
                ts: test_ts(),
                data,
                proof,
            },
        }
    }

    // bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY
    pub fn bill_id_test() -> BillId {
        BillId::new(
            PublicKey::from_str(
                "026423b7d36d05b8d50a89a1b4ef2a06c88bcd2c5e650f25e122fa682d3b39686c",
            )
            .unwrap(),
            bitcoin::Network::Testnet,
        )
    }

    pub fn private_key_test() -> SecretKey {
        SecretKey::from_str("d1ff7427912d3b81743d3b67ffa1e65df2156d3dab257316cbc8d0f35eeeabe9")
            .unwrap()
    }

    pub fn node_id_test() -> NodeId {
        NodeId::from_str("bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0")
            .unwrap()
    }

    pub fn node_id_test_other() -> NodeId {
        NodeId::from_str("bitcrt03f9f94d1fdc2090d46f3524807e3f58618c36988e69577d70d5d4d1e9e9645a4f")
            .unwrap()
    }

    pub fn node_id_test_another() -> NodeId {
        NodeId::from_str("bitcrt023827c9c6d3ff8de504c714997a2ad36efd761d09a8af5d5480f71199fcbb6098")
            .unwrap()
    }

    pub fn private_key_test_another() -> bitcoin::secp256k1::SecretKey {
        bitcoin::secp256k1::SecretKey::from_str(
            "f50032a6a67bc86f9542e74b7becc31847ff94d74e7760dcb797435d45463345",
        )
        .unwrap()
    }

    pub fn update_company_block_with_name(name: Option<Name>) -> CompanyUpdateBlockData {
        CompanyUpdateBlockData {
            name,
            email: Default::default(),
            country: Default::default(),
            city: Default::default(),
            zip: EditOptionalFieldMode::Ignore,
            address: Default::default(),
            country_of_registration: EditOptionalFieldMode::Ignore,
            city_of_registration: EditOptionalFieldMode::Ignore,
            registration_number: EditOptionalFieldMode::Ignore,
            registration_date: EditOptionalFieldMode::Ignore,
            logo_file: EditOptionalFieldMode::Ignore,
            proof_of_registration_file: EditOptionalFieldMode::Ignore,
        }
    }

    pub fn update_identity_block_with_name(name: Option<Name>) -> IdentityUpdateBlockData {
        IdentityUpdateBlockData {
            t: Default::default(),
            name,
            email: Default::default(),
            country: Default::default(),
            city: Default::default(),
            zip: EditOptionalFieldMode::Ignore,
            address: Default::default(),
            date_of_birth: EditOptionalFieldMode::Ignore,
            country_of_birth: EditOptionalFieldMode::Ignore,
            city_of_birth: EditOptionalFieldMode::Ignore,
            identification_number: EditOptionalFieldMode::Ignore,
            profile_picture_file: EditOptionalFieldMode::Ignore,
            identity_document_file: EditOptionalFieldMode::Ignore,
        }
    }
}
