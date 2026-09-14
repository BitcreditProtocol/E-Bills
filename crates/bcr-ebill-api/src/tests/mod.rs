#[cfg(test)]
#[allow(clippy::module_inception)]
pub mod tests {
    use crate::CourtConfig;
    use crate::service::transport_service::{self, chain_keys::ChainKeyServiceApi};
    use crate::{
        CONFIG, DbContext, DevModeConfig, MintConfig, NostrConfig, NostrContactStoreApi,
        PaymentConfig,
    };
    use async_trait::async_trait;
    use bcr_common::core::{BillId, NodeId};
    use bcr_ebill_core::protocol::Address;
    use bcr_ebill_core::protocol::BitcoinAddress;
    use bcr_ebill_core::protocol::BlockId;
    use bcr_ebill_core::protocol::City;
    use bcr_ebill_core::protocol::Country;
    use bcr_ebill_core::protocol::Date;
    use bcr_ebill_core::protocol::Email;
    use bcr_ebill_core::protocol::Name;
    use bcr_ebill_core::protocol::Sha256Hash;
    use bcr_ebill_core::protocol::Sum;
    use bcr_ebill_core::protocol::Timestamp;
    use bcr_ebill_core::protocol::blockchain::bill::BitcreditBill;
    use bcr_ebill_core::protocol::{EmailIdentityProofData, SignedIdentityProof};
    use bcr_ebill_core::{
        application::ServiceTraitBounds,
        application::bill::{BitcreditBillResult, PaymentState},
        application::company::{Company, LocalSignatoryOverride, LocalSignatoryOverrideStatus},
        application::contact::Contact,
        application::identity::{ActiveIdentityState, Identity, IdentityWithAll},
        application::nostr_contact::{HandshakeStatus, NostrContact, NostrPublicKey, TrustLevel},
        application::notification::{Notification, NotificationType},
        protocol::blockchain::{
            BlockchainType,
            bill::{
                BillBlock, BillBlockchain, BillOpCode, ContactType,
                participant::{BillIdentParticipant, BillParticipant},
            },
            company::{CompanyBlock, CompanyBlockchain},
            identity::{IdentityBlock, IdentityBlockchain, IdentityType},
        },
        protocol::crypto::BcrKeys,
        protocol::event::bill_events::ActionType,
        protocol::mint::{MintOffer, MintRequest, MintRequestStatus},
        protocol::{OptionalPostalAddress, PostalAddress, PublicKey, SecretKey},
    };
    use bcr_ebill_persistence::notification::EmailNotificationStoreApi;
    use bcr_ebill_persistence::{
        ContactStoreApi, FileReferenceStoreApi, NostrEventOffset, NostrEventOffsetStoreApi,
        NotificationStoreApi, PendingContactShare, Result, ShareDirection, SurrealDbConfig,
        bill::{BillChainStoreApi, BillStoreApi},
        company::{CompanyChainStoreApi, CompanyStoreApi},
        file_upload::FileUploadStoreApi,
        identity::{IdentityChainStoreApi, IdentityStoreApi},
        mint::MintStoreApi,
        nostr::{
            NostrChainEvent, NostrChainEventStoreApi, NostrQueuedMessage,
            NostrQueuedMessageStoreApi, RelaySyncStatus, SyncStatus,
        },
        notification::NotificationFilter,
    };
    use std::sync::Arc;
    use std::{
        collections::{HashMap, HashSet},
        str::FromStr,
    };
    use uuid::Uuid;

    // Need to wrap mocks, because traits are in a different crate
    mockall::mock! {
        pub ContactStoreApiMock {}

        impl ServiceTraitBounds for ContactStoreApiMock {}

        #[async_trait]
        impl ContactStoreApi for ContactStoreApiMock {
            async fn search(&self, search_term: &str) -> Result<Vec<Contact>>;
            async fn get_map(&self) -> Result<HashMap<NodeId, Contact>>;
            async fn get(&self, node_id: &NodeId) -> Result<Option<Contact>>;
            async fn insert(&self, node_id: &NodeId, data: Contact) -> Result<()>;
            async fn delete(&self, node_id: &NodeId) -> Result<()>;
            async fn update(&self, node_id: &NodeId, data: Contact) -> Result<()>;
        }
    }

    mockall::mock! {
        pub MintStore {}

        impl ServiceTraitBounds for MintStore {}

        #[async_trait]
        impl MintStoreApi for MintStore {
            async fn exists_for_bill(&self, requester_node_id: &NodeId, bill_id: &BillId) -> Result<bool>;
            async fn get_all_active_requests(&self) -> Result<Vec<MintRequest>>;
            async fn get_requests(
                &self,
                requester_node_id: &NodeId,
                bill_id: &BillId,
                mint_node_id: &NodeId,
            ) -> Result<Vec<MintRequest>>;
            async fn get_requests_for_bill(
                &self,
                requester_node_id: &NodeId,
                bill_id: &BillId,
            ) -> Result<Vec<MintRequest>>;
            async fn add_request(
                &self,
                requester_node_id: &NodeId,
                bill_id: &BillId,
                mint_node_id: &NodeId,
                mint_request_id: &Uuid,
                timestamp: Timestamp,
            ) -> Result<()>;
            async fn get_request(&self, mint_request_id: &Uuid) -> Result<Option<MintRequest>>;
            async fn update_request(
                &self,
                mint_request_id: &Uuid,
                new_status: &MintRequestStatus,
            ) -> Result<()>;
            async fn add_proofs_to_offer(&self, mint_request_id: &Uuid, proofs: &str) -> Result<()>;
            async fn add_recovery_data_to_offer(
                &self,
                mint_request_id: &Uuid,
                secrets: &[String],
                rs: &[String],
            ) -> Result<()>;
            async fn set_proofs_to_spent_for_offer(&self, mint_request_id: &Uuid) -> Result<()>;
            async fn add_offer(
                &self,
                mint_request_id: &Uuid,
                keyset_id: &str,
                expiration_timestamp: Timestamp,
                discounted_sum: Sum,
            ) -> Result<()>;
            async fn get_offer(&self, mint_request_id: &Uuid) -> Result<Option<MintOffer>>;
        }
    }

    mockall::mock! {
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

    mockall::mock! {
        pub BillStoreApiMock {}

        impl ServiceTraitBounds for BillStoreApiMock {}

        #[async_trait]
        impl BillStoreApi for BillStoreApiMock {
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
                op_code: HashSet<BillOpCode>,
                since: Timestamp,
            ) -> Result<Vec<BillId>>;
        }
    }

    mockall::mock! {
        pub BillChainStoreApiMock {}

        impl ServiceTraitBounds for BillChainStoreApiMock {}

        #[async_trait]
        impl BillChainStoreApi for BillChainStoreApiMock {
            async fn get_latest_block(&self, id: &BillId) -> Result<BillBlock>;
            async fn add_block(&self, id: &BillId, block: &BillBlock) -> Result<()>;
            async fn get_chain(&self, id: &BillId) -> Result<BillBlockchain>;
            async fn remove_blocks_from_height(&self, id: &BillId, from_block_id: BlockId) -> Result<()>;
        }
    }

    mockall::mock! {
        pub ChainKeyService {}

        impl ServiceTraitBounds for ChainKeyService {}

        #[async_trait]
        impl ChainKeyServiceApi for ChainKeyService {
            async fn get_chain_keys(&self, chain_id: &str, chain_type: BlockchainType) -> transport_service::Result<Option<BcrKeys>>;
        }
    }

    mockall::mock! {
        pub CompanyStoreApiMock {}

        impl ServiceTraitBounds for CompanyStoreApiMock {}

        #[async_trait]
        impl CompanyStoreApi for CompanyStoreApiMock {
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

    mockall::mock! {
        pub CompanyChainStoreApiMock {}

        impl ServiceTraitBounds for CompanyChainStoreApiMock {}

        #[async_trait]
        impl CompanyChainStoreApi for CompanyChainStoreApiMock {
            async fn get_latest_block(&self, id: &NodeId) -> Result<CompanyBlock>;
            async fn add_block(&self, id: &NodeId, block: &CompanyBlock) -> Result<()>;
            async fn remove(&self, id: &NodeId) -> Result<()>;
            async fn get_chain(&self, id: &NodeId) -> Result<CompanyBlockchain>;
            async fn remove_blocks_from_height(&self, id: &NodeId, from_block_id: BlockId) -> Result<()>;
        }
    }

    mockall::mock! {
        pub IdentityStoreApiMock {}

        impl ServiceTraitBounds for IdentityStoreApiMock {}

        #[async_trait]
        impl IdentityStoreApi for IdentityStoreApiMock {
            async fn exists(&self) -> bool;
            async fn save(&self, identity: &Identity) -> Result<()>;
            async fn get(&self) -> Result<Identity>;
            async fn get_full(&self) -> Result<IdentityWithAll>;
            async fn save_key_pair(&self, key_pair: &BcrKeys, seed: &str) -> Result<()>;
            async fn get_key_pair(&self) -> Result<BcrKeys>;
            async fn get_or_create_key_pair(&self) -> Result<BcrKeys>;
            async fn get_seedphrase(&self) -> Result<String>;
            async fn get_current_identity(&self) -> Result<ActiveIdentityState>;
            async fn set_current_identity(&self, identity_state: &ActiveIdentityState) -> Result<()>;
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

    mockall::mock! {
        pub IdentityChainStoreApiMock {}

        impl ServiceTraitBounds for IdentityChainStoreApiMock {}

        #[async_trait]
        impl IdentityChainStoreApi for IdentityChainStoreApiMock {
            async fn get_latest_block(&self) -> Result<IdentityBlock>;
            async fn add_block(&self, block: &IdentityBlock) -> Result<()>;
            async fn get_chain(&self) -> Result<IdentityBlockchain>;
            async fn remove_blocks_from_height(&self, from_block_id: BlockId) -> Result<()>;
        }
    }

    mockall::mock! {
        pub NostrEventOffsetStoreApiMock {}

        impl ServiceTraitBounds for NostrEventOffsetStoreApiMock {}

        #[async_trait]
        impl NostrEventOffsetStoreApi for NostrEventOffsetStoreApiMock {
            async fn current_offset(&self, node_id: &NodeId) -> Result<Timestamp>;
            async fn is_processed(&self, event_id: &str) -> Result<bool>;
            async fn add_event(&self, data: NostrEventOffset) -> Result<()>;
        }
    }

    mockall::mock! {
        pub NostrQueuedMessageStore {}

        impl ServiceTraitBounds for NostrQueuedMessageStore {}

        #[async_trait]
        impl NostrQueuedMessageStoreApi for NostrQueuedMessageStore {
            async fn add_message(&self, message: NostrQueuedMessage, max_retries: i32) -> Result<()>;
            async fn get_retry_messages(&self, limit: u64) -> Result<Vec<NostrQueuedMessage>>;
            async fn fail_retry(&self, id: &str) -> Result<()>;
            async fn succeed_retry(&self, id: &str) -> Result<()>;
        }
    }

    mockall::mock! {
        pub NostrChainEventStore {}

        impl ServiceTraitBounds for NostrChainEventStore {}

        #[async_trait]
        impl NostrChainEventStoreApi for NostrChainEventStore {
            async fn find_chain_events(&self, chain_id: &str, chain_type: BlockchainType) -> Result<Vec<NostrChainEvent>>;
            async fn find_latest_block_events(&self, chain_id: &str, chain_type: BlockchainType) -> Result<Vec<NostrChainEvent>>;
            async fn find_root_event(&self,chain_id: &str, chain_type: BlockchainType) -> Result<Option<NostrChainEvent>>;
            async fn find_by_block_hash(&self, hash: &Sha256Hash) -> Result<Option<NostrChainEvent>>;
            async fn add_chain_event(&self, event: NostrChainEvent) -> Result<()>;
            async fn by_event_id(&self, event_id: &str) -> Result<Option<NostrChainEvent>>;
            async fn remove_chain_events(&self, chain_id: &str, chain_type: BlockchainType) -> Result<()>;
        }
    }

    mockall::mock! {
        pub NotificationStoreApiMock {}

        impl ServiceTraitBounds for NotificationStoreApiMock {}

        #[async_trait]
        impl NotificationStoreApi for NotificationStoreApiMock {
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
            async fn list_by_type(&self, notification_type: NotificationType) -> Result<Vec<Notification>>;
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

    mockall::mock! {
        pub EmailNotificationStoreApiMock {}

        impl ServiceTraitBounds for EmailNotificationStoreApiMock {}

        #[async_trait]
        impl EmailNotificationStoreApi for EmailNotificationStoreApiMock {
            async fn add_email_preferences_link_for_node_id(
                &self,
                email_preferences_link: &url::Url,
                node_id: &NodeId,
            ) -> Result<()>;
            async fn get_email_preferences_link_for_node_id(&self, node_id: &NodeId) -> Result<Option<url::Url>>;
        }
    }

    mockall::mock! {
        pub FileUploadStoreApiMock {}

        impl ServiceTraitBounds for FileUploadStoreApiMock {}

        #[async_trait]
        impl FileUploadStoreApi for FileUploadStoreApiMock {
            async fn remove_temp_upload_folder(&self, file_upload_id: &Uuid) -> Result<()>;
            async fn write_temp_upload_file(
                &self,
                file_upload_id: &Uuid,
                file_name: &Name,
                file_bytes: &[u8],
            ) -> Result<()>;
            async fn read_temp_upload_file(&self, file_upload_id: &Uuid) -> Result<(Name, Vec<u8>)>;
        }
    }

    mockall::mock! {
        pub FileReferenceStoreApiMock {}

        impl ServiceTraitBounds for FileReferenceStoreApiMock {}

        #[async_trait]
        impl FileReferenceStoreApi for FileReferenceStoreApiMock {
            async fn upsert(
                &self,
                hash: &Sha256Hash,
                nostr_hash: &bitcoin::hashes::sha256::Hash,
                name: Option<Name>,
                server_urls: Vec<url::Url>,
                is_important: Option<bool>,
                context: Vec<bcr_ebill_core::protocol::file_reference::FileReferenceContext>,
            ) -> bcr_ebill_persistence::Result<bcr_ebill_core::protocol::file_reference::FileReference>;
            async fn get(&self, hash: &Sha256Hash) -> bcr_ebill_persistence::Result<Option<bcr_ebill_core::protocol::file_reference::FileReference>>;
            async fn find_by_nostr_hash(&self, nostr_hash: &bitcoin::hashes::sha256::Hash) -> bcr_ebill_persistence::Result<Option<bcr_ebill_core::protocol::file_reference::FileReference>>;
            async fn delete(&self, hash: &Sha256Hash) -> bcr_ebill_persistence::Result<()>;
            async fn list(&self) -> bcr_ebill_persistence::Result<Vec<bcr_ebill_core::protocol::file_reference::FileReference>>;
            async fn list_important(&self) -> bcr_ebill_persistence::Result<Vec<bcr_ebill_core::protocol::file_reference::FileReference>>;
            async fn add_server_urls(&self, hash: &Sha256Hash, urls: Vec<url::Url>) -> bcr_ebill_persistence::Result<bool>;
            async fn mark_important(&self, hash: &Sha256Hash, important: bool) -> bcr_ebill_persistence::Result<()>;
            async fn update_nostr_hash(&self, hash: &Sha256Hash, nostr_hash: &bitcoin::hashes::sha256::Hash) -> bcr_ebill_persistence::Result<()>;
            async fn add_context(&self, hash: &Sha256Hash, context: bcr_ebill_core::protocol::file_reference::FileReferenceContext) -> bcr_ebill_persistence::Result<bool>;
            async fn remove_context(&self, hash: &Sha256Hash, context: &bcr_ebill_core::protocol::file_reference::FileReferenceContext) -> bcr_ebill_persistence::Result<bool>;
        }
    }

    #[allow(unused)]
    pub fn get_mock_db_ctx(nostr_contact_store: Option<MockNostrContactStore>) -> DbContext {
        DbContext {
            contact_store: Arc::new(MockContactStoreApiMock::new()),
            bill_store: Arc::new(MockBillStoreApiMock::new()),
            bill_blockchain_store: Arc::new(MockBillChainStoreApiMock::new()),
            identity_store: Arc::new(MockIdentityStoreApiMock::new()),
            identity_chain_store: Arc::new(MockIdentityChainStoreApiMock::new()),
            company_chain_store: Arc::new(MockCompanyChainStoreApiMock::new()),
            company_store: Arc::new(MockCompanyStoreApiMock::new()),
            file_upload_store: Arc::new(MockFileUploadStoreApiMock::new()),
            file_reference_store: Arc::new(MockFileReferenceStoreApiMock::new()),
            nostr_event_offset_store: Arc::new(MockNostrEventOffsetStoreApiMock::new()),
            notification_store: Arc::new(MockNotificationStoreApiMock::new()),
            email_notification_store: Arc::new(MockEmailNotificationStoreApiMock::new()),
            queued_message_store: Arc::new(MockNostrQueuedMessageStore::new()),
            nostr_contact_store: Arc::new(nostr_contact_store.unwrap_or_default()),
            mint_store: Arc::new(MockMintStore::new()),
            nostr_chain_event_store: Arc::new(MockNostrChainEventStore::new()),
        }
    }

    pub fn init_test_cfg() {
        match CONFIG.get() {
            Some(_) => (),
            None => {
                let _ = crate::init(crate::Config {
                    bitcoin_network: "testnet".to_string(),
                    esplora_base_urls: vec![
                        url::Url::parse("https://esplora.minibill.tech").unwrap(),
                    ],
                    db_config: SurrealDbConfig {
                        connection_string: "ws://localhost:8800".to_string(),
                        ..SurrealDbConfig::default()
                    },
                    files_db_config: SurrealDbConfig {
                        connection_string: "ws://localhost:8800".to_string(),
                        ..SurrealDbConfig::default()
                    },
                    nostr_config: NostrConfig {
                        only_known_contacts: false,
                        relays: vec![url::Url::parse("ws://localhost:8080").unwrap()],
                        blossom_servers: vec![],
                        max_relays: Some(50),
                        relay_ack_threshold: 1,
                    },
                    mint_config: MintConfig {
                        default_mint_url: url::Url::parse("http://localhost:4242/").unwrap(),
                        default_mint_node_id: node_id_test(),
                    },
                    payment_config: PaymentConfig {
                        num_confirmations_for_payment: 6,
                    },
                    dev_mode_config: DevModeConfig {
                        on: false,
                        mandatory_email_confirmations: true,
                    },
                    court_config: CourtConfig {
                        default_url: url::Url::parse("https://court-dev.minibill.tech").unwrap(),
                    },
                });
            }
        }
    }

    pub fn empty_address() -> PostalAddress {
        PostalAddress {
            country: Country::AT,
            city: City::new("Vienna").unwrap(),
            zip: None,
            address: Address::new("Some Address 1").unwrap(),
        }
    }

    pub fn filled_optional_address() -> OptionalPostalAddress {
        OptionalPostalAddress {
            country: Some(Country::AT),
            city: Some(City::new("Vienna").unwrap()),
            zip: None,
            address: Some(Address::new("Some Address 1").unwrap()),
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

    pub fn empty_other_identity() -> Identity {
        Identity {
            t: IdentityType::Ident,
            node_id: node_id_test_another(),
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

    pub fn bill_participant_only_node_id(node_id: NodeId) -> BillParticipant {
        BillParticipant::Ident(BillIdentParticipant {
            t: ContactType::Person,
            node_id,
            name: Name::new("some name").unwrap(),
            postal_address: empty_address(),
            email: None,
            nostr_relays: vec![],
        })
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
            sum: Sum::new_sat(5000).expect("sat works"),
            maturity_date: Date::new("2099-11-12").unwrap(),
            issue_date: Date::new("2099-08-12").unwrap(),
            city_of_payment: City::new("Vienna").unwrap(),
            country_of_payment: Country::AT,
            files: vec![],
        }
    }

    pub fn bill_identified_participant_only_node_id(node_id: NodeId) -> BillIdentParticipant {
        BillIdentParticipant {
            t: ContactType::Person,
            node_id,
            name: Name::new("some name").unwrap(),
            postal_address: empty_address(),
            email: None,
            nostr_relays: vec![],
        }
    }

    pub fn node_id_test() -> NodeId {
        NodeId::from_str("bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0")
            .unwrap()
    }

    pub fn node_id_test_other() -> NodeId {
        NodeId::from_str("bitcrt03f9f94d1fdc2090d46f3524807e3f58618c36988e69577d70d5d4d1e9e9645a4f")
            .unwrap()
    }

    pub fn node_id_test_other2() -> NodeId {
        NodeId::from_str("bitcrt039180c169e5f6d7c579cf1cefa37bffd47a2b389c8125601f4068c87bea795943")
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

    pub fn signed_identity_proof_test() -> (SignedIdentityProof, EmailIdentityProofData) {
        let data = EmailIdentityProofData {
            node_id: node_id_test(),
            company_node_id: None,
            email: Email::new("test@example.com").unwrap(),
            created_at: test_ts(),
        };
        let proof = data.sign(&node_id_test(), &private_key_test()).unwrap();
        (proof, data)
    }

    pub fn signed_other_identity_proof_test() -> (SignedIdentityProof, EmailIdentityProofData) {
        let data = EmailIdentityProofData {
            node_id: node_id_test_another(),
            company_node_id: None,
            email: Email::new("test@example.com").unwrap(),
            created_at: test_ts(),
        };
        let proof = data.sign(&node_id_test(), &private_key_test()).unwrap();
        (proof, data)
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

    // bitcrt76LWp9iFregj9Lv1awLSfQAmjtDDinBR4GSCbNrEtqEe
    pub fn bill_id_test_other() -> BillId {
        BillId::new(
            PublicKey::from_str(
                "027a233c85a8f98e276e949ab94bba8bbc07b21946e50e388da767bcc6c95603ce",
            )
            .unwrap(),
            bitcoin::Network::Testnet,
        )
    }

    // bitcrtJArd6A7fDhkiD3AU5UgBSQ66yjzQT1NP9tVoeQ1aZW1y
    pub fn bill_id_test_other2() -> BillId {
        BillId::new(
            PublicKey::from_str(
                "0364f8de530163a528b4de33405ebe434bbd974a26ac24708674de572efacbdfdd",
            )
            .unwrap(),
            bitcoin::Network::Testnet,
        )
    }

    pub fn private_key_test() -> SecretKey {
        SecretKey::from_str("d1ff7427912d3b81743d3b67ffa1e65df2156d3dab257316cbc8d0f35eeeabe9")
            .unwrap()
    }

    pub const NODE_ID_TEST_STR: &str =
        "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0";

    pub fn valid_payment_address_testnet() -> BitcoinAddress {
        BitcoinAddress::from_str("tb1qteyk7pfvvql2r2zrsu4h4xpvju0nz7ykvguyk0").unwrap()
    }

    pub fn test_ts() -> Timestamp {
        Timestamp::new(1731593928).unwrap()
    }

    #[cfg(test)]
    mod config_tests {
        use crate::NostrConfig;

        #[test]
        fn test_nostr_config_default_max_relays() {
            let config = NostrConfig::default();
            assert_eq!(config.max_relays, Some(50));
        }

        #[test]
        fn test_nostr_config_default_threshold() {
            let config = NostrConfig::default();
            assert_eq!(config.relay_ack_threshold, 1);
        }

        #[test]
        fn test_nostr_config_with_custom_max_relays() {
            let config = NostrConfig {
                only_known_contacts: true,
                relays: vec![],
                blossom_servers: vec![],
                max_relays: Some(100),
                relay_ack_threshold: 1,
            };
            assert_eq!(config.max_relays, Some(100));
        }

        #[test]
        fn test_nostr_config_with_no_relay_limit() {
            let config = NostrConfig {
                only_known_contacts: false,
                relays: vec![],
                blossom_servers: vec![],
                max_relays: None,
                relay_ack_threshold: 2,
            };
            assert_eq!(config.max_relays, None);
            assert_eq!(config.relay_ack_threshold, 2);
        }
    }
}
