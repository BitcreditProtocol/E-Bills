use std::sync::Arc;

use crate::handler::public_chain_helpers::{BlockData, resolve_event_chains};
use crate::handler::{
    BillChainEventProcessorApi, CompanyChainEventProcessorApi, IdentityChainEventProcessorApi,
};
use crate::nostr_transport::NostrTransportService;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_api::service::transport_service::BlockTransportServiceApi;
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::blockchain::BlockchainType;
use bcr_ebill_core::protocol::blockchain::bill::BillBlock;
use bcr_ebill_core::protocol::event::{
    BillChainEvent, CompanyChainEvent, EventEnvelope, IdentityChainEvent,
};
use bcr_ebill_persistence::nostr::NostrChainEvent;
use bitcoin::base58;
use log::{debug, error};

use bcr_ebill_api::service::transport_service::{Error, Result};

#[derive(Clone)]
pub struct BlockTransportService {
    nostr_transport: Arc<NostrTransportService>,
    bill_chain_event_processor: Arc<dyn BillChainEventProcessorApi>,
    company_chain_event_processor: Arc<dyn CompanyChainEventProcessorApi>,
    identity_chain_event_processor: Arc<dyn IdentityChainEventProcessorApi>,
}

impl BlockTransportService {
    pub fn new(
        nostr_transport: Arc<NostrTransportService>,
        bill_chain_event_processor: Arc<dyn BillChainEventProcessorApi>,
        company_chain_event_processor: Arc<dyn CompanyChainEventProcessorApi>,
        identity_chain_event_processor: Arc<dyn IdentityChainEventProcessorApi>,
    ) -> Self {
        Self {
            nostr_transport,
            bill_chain_event_processor,
            company_chain_event_processor,
            identity_chain_event_processor,
        }
    }
}

impl ServiceTraitBounds for BlockTransportService {}

impl BlockTransportService {
    /// Validates that a previous block event exists before publishing.
    /// If no previous event is found and the block is not genesis,
    /// returns an error to prevent publishing an orphaned block.
    async fn validate_previous_event_exists(
        &self,
        previous_hash: &Sha256Hash,
        chain_id: &str,
        chain_type: BlockchainType,
        block_height: usize,
    ) -> Result<(Option<NostrChainEvent>, Option<NostrChainEvent>)> {
        let (previous_event, root_event) = self
            .nostr_transport
            .find_root_and_previous_event(previous_hash, chain_id, chain_type)
            .await?;

        if previous_event.is_none() && block_height > 1 {
            return Err(Error::Blockchain(format!(
                "Cannot publish block: missing previous block for {chain_type:?} chain {chain_id} at height {block_height}"
            )));
        }

        Ok((previous_event, root_event))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl BlockTransportServiceApi for BlockTransportService {
    /// Sent when an identity chain is created or updated
    async fn send_identity_chain_events(&self, events: IdentityChainEvent) -> Result<()> {
        debug!(
            "sending identity chain events for node: {}",
            events.identity_id
        );
        let node = self.nostr_transport.get_node_transport(&events.sender());

        if let Some(event) = events.generate_blockchain_message() {
            let (previous_event, root_event) = self
                .validate_previous_event_exists(
                    &event.data.block.previous_hash,
                    &event.data.node_id.to_string(),
                    BlockchainType::Identity,
                    event.data.block.id.inner() as usize,
                )
                .await?;
            let nostr_event = node
                .build_public_chain_event(
                    &events.sender(),
                    &event.data.node_id.to_string(),
                    BlockchainType::Identity,
                    event.data.block.timestamp,
                    event.clone().try_into()?,
                    previous_event.clone().map(|e| e.payload),
                    root_event.clone().map(|e| e.payload),
                )
                .await?;

            let threshold = node.relay_ack_threshold();
            if let Err(e) = node
                .broadcast_event_optimistic(&nostr_event, threshold)
                .await
            {
                error!("Failed to broadcast identity chain event, queuing for retry: {e}");
                let payload = serde_json::to_string(&nostr_event)
                    .map_err(|e| Error::Message(e.to_string()))?;
                self.nostr_transport
                    .queue_retry_message_and_trigger(&events.sender(), None, payload)
                    .await?;
            }

            self.nostr_transport
                .add_chain_event(
                    &nostr_event,
                    &root_event,
                    &previous_event,
                    &event.data.node_id.to_string(),
                    BlockchainType::Identity,
                    event.data.block.id.inner() as usize,
                    &event.data.block.hash,
                )
                .await?;
        }

        Ok(())
    }

    /// Sent when a company chain is created or updated
    async fn send_company_chain_events(&self, events: CompanyChainEvent) -> Result<()> {
        debug!(
            "sending company chain events for company id: {}",
            events.company_id
        );
        let node = self.nostr_transport.get_node_transport(&events.sender());

        if let Some(event) = events.generate_blockchain_message() {
            let (previous_event, root_event) = self
                .validate_previous_event_exists(
                    &event.data.block.previous_hash,
                    &event.data.node_id.to_string(),
                    BlockchainType::Company,
                    event.data.block.id.inner() as usize,
                )
                .await?;
            let nostr_event = node
                .build_public_chain_event(
                    &events.sender(),
                    &event.data.node_id.to_string(),
                    BlockchainType::Company,
                    event.data.block.timestamp,
                    event.clone().try_into()?,
                    previous_event.clone().map(|e| e.payload),
                    root_event.clone().map(|e| e.payload),
                )
                .await?;

            let threshold = node.relay_ack_threshold();
            if let Err(e) = node
                .broadcast_event_optimistic(&nostr_event, threshold)
                .await
            {
                error!("Failed to broadcast company chain event, queuing for retry: {e}");
                let payload = serde_json::to_string(&nostr_event)
                    .map_err(|e| Error::Message(e.to_string()))?;
                self.nostr_transport
                    .queue_retry_message_and_trigger(&events.sender(), None, payload)
                    .await?;
            }

            self.nostr_transport
                .add_chain_event(
                    &nostr_event,
                    &root_event,
                    &previous_event,
                    &event.data.node_id.to_string(),
                    BlockchainType::Company,
                    event.data.block.id.inner() as usize,
                    &event.data.block.hash,
                )
                .await?;
        }

        // handle potential invite for new signatory
        if let Some((recipient, invite)) = events.generate_company_invite_message()
            && let Some(identity) = self.nostr_transport.resolve_identity(&recipient).await
        {
            let message: EventEnvelope = invite.try_into()?;
            if let Err(e) = node
                .send_private_event(&events.sender(), &identity, message.clone())
                .await
            {
                error!("Failed to send company invite, queuing for retry: {e}");
                self.nostr_transport
                    .queue_retry_message_and_trigger(
                        &events.sender(),
                        Some(&recipient),
                        base58::encode(&borsh::to_vec(&message)?),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    /// Sent when: A bill chain is created or updated
    async fn send_bill_chain_events(&self, events: BillChainEvent) -> Result<()> {
        let node = self.nostr_transport.get_node_transport(&events.sender());

        if let Some(block_event) = events.generate_blockchain_message() {
            let (previous_event, root_event) = self
                .validate_previous_event_exists(
                    &block_event.data.block.previous_hash,
                    &block_event.data.bill_id.to_string(),
                    BlockchainType::Bill,
                    block_event.data.block.id.inner() as usize,
                )
                .await?;

            let nostr_event = node
                .build_public_chain_event(
                    &events.sender(),
                    &block_event.data.bill_id.to_string(),
                    BlockchainType::Bill,
                    block_event.data.block.timestamp,
                    block_event.clone().try_into()?,
                    previous_event.clone().map(|e| e.payload),
                    root_event.clone().map(|e| e.payload),
                )
                .await?;

            let threshold = node.relay_ack_threshold();
            if let Err(e) = node
                .broadcast_event_optimistic(&nostr_event, threshold)
                .await
            {
                error!("Failed to broadcast bill chain event, queuing for retry: {e}");
                let payload = serde_json::to_string(&nostr_event)
                    .map_err(|e| Error::Message(e.to_string()))?;
                self.nostr_transport
                    .queue_retry_message_and_trigger(&events.sender(), None, payload)
                    .await?;
            }

            self.nostr_transport
                .add_chain_event(
                    &nostr_event,
                    &root_event,
                    &previous_event,
                    &block_event.data.bill_id.to_string(),
                    BlockchainType::Bill,
                    block_event.data.block.id.inner() as usize,
                    &block_event.data.block.hash,
                )
                .await?;
        }

        let invites = events.generate_bill_invite_events();
        if !invites.is_empty() {
            for (recipient, event) in invites {
                if let Some(identity) = self.nostr_transport.resolve_identity(&recipient).await {
                    let message: EventEnvelope = event.try_into()?;
                    if let Err(e) = node
                        .send_private_event(&events.sender(), &identity, message.clone())
                        .await
                    {
                        error!("Failed to send bill invite, queuing for retry: {e}");
                        self.nostr_transport
                            .queue_retry_message_and_trigger(
                                &events.sender(),
                                Some(&recipient),
                                base58::encode(&borsh::to_vec(&message)?),
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Resync bill chain. If `from_nostr` is true, fetches missing blocks from Nostr first.
    /// If false, only invalidates the local cache.
    async fn resync_bill_chain(&self, bill_id: &BillId, from_nostr: bool) -> Result<()> {
        self.bill_chain_event_processor
            .resync_chain(bill_id, from_nostr)
            .await?;
        Ok(())
    }

    /// Resync company chain
    async fn resync_company_chain(&self, company_id: &NodeId) -> Result<()> {
        self.company_chain_event_processor
            .resync_chain(company_id)
            .await?;
        Ok(())
    }

    /// Resync identity chain
    async fn resync_identity_chain(&self) -> Result<()> {
        self.identity_chain_event_processor.resync_chain().await?;
        Ok(())
    }

    /// Fetch all chains for this bill id from nostr and attempt to find one which exactly matches the given blocks
    async fn validate_bill_blocks_exist_on_nostr_chain(
        &self,
        bill_id: &BillId,
        blocks: &[BillBlock],
    ) -> Result<bool> {
        if blocks.is_empty() {
            return Ok(true);
        }
        let mut sorted_blocks = blocks.to_vec();
        sorted_blocks.sort_by(|x, y| x.bill_id.cmp(&y.bill_id).then_with(|| x.id.cmp(&y.id)));

        let transport = self.nostr_transport.get_first_transport();
        // chains are sorted by longest first, so we start there and move down until we have a success
        let chains =
            resolve_event_chains(transport, &bill_id.to_string(), BlockchainType::Bill, &None)
                .await?;
        for chain in chains.iter() {
            // ignore empty chains
            if chain.is_empty() {
                continue;
            }

            let mut chain_blocks: Vec<BillBlock> = chain
                .iter()
                .filter_map(|d| match d.block.clone() {
                    BlockData::Bill(block) => Some(block),
                    _ => None,
                })
                .collect();
            chain_blocks.sort_by(|x, y| x.bill_id.cmp(&y.bill_id).then_with(|| x.id.cmp(&y.id)));

            // ignore chains with no bill blocks
            if chain_blocks.is_empty() {
                continue;
            }

            // chain has to match exactly
            if chain_blocks.len() != sorted_blocks.len() {
                continue;
            }

            // if it matches exactly, it's valid
            if sorted_blocks == chain_blocks {
                return Ok(true);
            }
        }
        // didn't find all blocks exactly in any chain - invalid
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::{
        MockBillChainEventProcessorApi, MockCompanyChainEventProcessorApi,
        MockIdentityChainEventProcessorApi,
    };
    use crate::test_utils::{
        MockContactStore, MockNostrChainEventStore, MockNostrContactStore,
        MockNostrQueuedMessageStore, MockNotificationJsonTransport, bill_id_test,
        get_genesis_chain, get_nostr_transport, get_test_company_chain_event,
        get_test_identity_chain_event, private_key_test,
    };
    use crate::transport::create_public_chain_event;
    use bcr_ebill_core::protocol::blockchain::{Blockchain, BlockchainType};
    use bcr_ebill_core::protocol::crypto::BcrKeys;
    use bcr_ebill_core::protocol::event::{BillBlockEvent, Event};
    use bcr_ebill_core::protocol::{BlockId, Sha256Hash, Timestamp};
    use bcr_ebill_persistence::nostr::NostrChainEvent;
    use nostr::event::FinalizeEvent;

    fn create_test_chain_event(
        chain_id: &str,
        chain_type: BlockchainType,
        block_height: usize,
        block_hash: Sha256Hash,
    ) -> NostrChainEvent {
        NostrChainEvent {
            event_id: format!("test_event_{block_height}"),
            root_id: "test_event_1".to_string(),
            reply_id: if block_height > 1 {
                Some(format!("test_event_{}", block_height - 1))
            } else {
                None
            },
            author: "test_author".to_string(),
            chain_id: chain_id.to_string(),
            chain_type,
            block_height,
            block_hash,
            received: bcr_ebill_core::protocol::Timestamp::now(),
            time: bcr_ebill_core::protocol::Timestamp::now(),
            payload: nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
                .finalize(&nostr::key::Keys::generate())
                .unwrap(),
        }
    }

    fn get_service_with_transport(
        mock_transport: MockNotificationJsonTransport,
        chain_event_store: MockNostrChainEventStore,
    ) -> BlockTransportService {
        BlockTransportService::new(
            Arc::new(get_nostr_transport(
                mock_transport,
                MockContactStore::new(),
                MockNostrContactStore::new(),
                MockNostrQueuedMessageStore::new(),
                chain_event_store,
            )),
            Arc::new(MockBillChainEventProcessorApi::new()),
            Arc::new(MockCompanyChainEventProcessorApi::new()),
            Arc::new(MockIdentityChainEventProcessorApi::new()),
        )
    }

    fn get_service(chain_event_store: MockNostrChainEventStore) -> BlockTransportService {
        get_service_with_transport(MockNotificationJsonTransport::new(), chain_event_store)
    }

    #[tokio::test]
    async fn test_validate_previous_event_exists_allows_genesis() {
        let mut chain_event_store = MockNostrChainEventStore::new();
        // No previous event in store for genesis block
        chain_event_store
            .expect_find_by_block_hash()
            .returning(|_| Ok(None));

        let service = get_service(chain_event_store);
        let result = service
            .validate_previous_event_exists(
                &Sha256Hash::new("genesis"),
                "test_chain",
                BlockchainType::Bill,
                1,
            )
            .await;

        assert!(result.is_ok());
        let (previous, root) = result.unwrap();
        assert!(previous.is_none());
        assert!(root.is_none());
    }

    #[tokio::test]
    async fn test_validate_previous_event_exists_rejects_missing() {
        let mut chain_event_store = MockNostrChainEventStore::new();
        // No previous event in store for non-genesis block
        chain_event_store
            .expect_find_by_block_hash()
            .returning(|_| Ok(None));

        let service = get_service(chain_event_store);
        let result = service
            .validate_previous_event_exists(
                &Sha256Hash::new("missing_hash"),
                "test_chain",
                BlockchainType::Bill,
                2,
            )
            .await;

        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Cannot publish block"));
        assert!(err_msg.contains("missing previous block"));
    }

    #[tokio::test]
    async fn test_validate_previous_event_exists_accepts_with_previous() {
        let mut chain_event_store = MockNostrChainEventStore::new();
        let previous_hash = Sha256Hash::new("previous_hash");
        let previous_event =
            create_test_chain_event("test_chain", BlockchainType::Bill, 1, previous_hash.clone());

        chain_event_store
            .expect_find_by_block_hash()
            .returning(move |_| Ok(Some(previous_event.clone())));

        let service = get_service(chain_event_store);
        let result = service
            .validate_previous_event_exists(&previous_hash, "test_chain", BlockchainType::Bill, 2)
            .await;

        assert!(result.is_ok());
        let (previous, root) = result.unwrap();
        assert!(previous.is_some());
        assert!(root.is_some());
    }

    #[tokio::test]
    async fn test_identity_chain_event_uses_optimistic_broadcast() {
        let signed_event =
            nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "identity chain event")
                .finalize(&nostr::key::Keys::generate())
                .unwrap();
        let mut transport = MockNotificationJsonTransport::new();
        transport
            .expect_build_public_chain_event()
            .returning(move |_, _, _, _, _, _, _| Ok(signed_event.clone()));
        transport.expect_relay_ack_threshold().returning(|| 1);
        transport
            .expect_broadcast_event_optimistic()
            .withf(|_event, min_acks| *min_acks == 1)
            .returning(|_, _| Ok(()));

        let mut chain_event_store = MockNostrChainEventStore::new();
        chain_event_store
            .expect_find_by_block_hash()
            .returning(|_| Ok(None));
        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()));

        let service = get_service_with_transport(transport, chain_event_store);
        let event = get_test_identity_chain_event();

        let result = service.send_identity_chain_events(event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_company_chain_event_uses_optimistic_broadcast() {
        let signed_event =
            nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "company chain event")
                .finalize(&nostr::key::Keys::generate())
                .unwrap();
        let mut transport = MockNotificationJsonTransport::new();
        transport
            .expect_build_public_chain_event()
            .returning(move |_, _, _, _, _, _, _| Ok(signed_event.clone()));
        transport.expect_relay_ack_threshold().returning(|| 1);
        transport
            .expect_broadcast_event_optimistic()
            .withf(|_event, min_acks| *min_acks == 1)
            .returning(|_, _| Ok(()));

        let mut chain_event_store = MockNostrChainEventStore::new();
        chain_event_store
            .expect_find_by_block_hash()
            .returning(|_| Ok(None));
        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()));

        let service = get_service_with_transport(transport, chain_event_store);
        let event = get_test_company_chain_event();

        let result = service.send_company_chain_events(event).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_broadcast_error_triggers_full_message_retry() {
        let signed_event =
            nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "retry event")
                .finalize(&nostr::key::Keys::generate())
                .unwrap();
        let mut transport = MockNotificationJsonTransport::new();
        transport
            .expect_build_public_chain_event()
            .returning(move |_, _, _, _, _, _, _| Ok(signed_event.clone()));
        transport.expect_relay_ack_threshold().returning(|| 1);
        transport
            .expect_broadcast_event_optimistic()
            .withf(|_event, min_acks| *min_acks == 1)
            .returning(|_, _| Err(Error::Network("relay failed".to_string())));

        let mut chain_event_store = MockNostrChainEventStore::new();
        chain_event_store
            .expect_find_by_block_hash()
            .returning(|_| Ok(None));
        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()));

        let mut queued_message_store = MockNostrQueuedMessageStore::new();
        queued_message_store
            .expect_add_message()
            .times(1)
            .returning(|_, _| Ok(()));
        queued_message_store
            .expect_get_retry_messages()
            .returning(|_| Ok(vec![]));

        let service = BlockTransportService::new(
            Arc::new(get_nostr_transport(
                transport,
                MockContactStore::new(),
                MockNostrContactStore::new(),
                queued_message_store,
                chain_event_store,
            )),
            Arc::new(MockBillChainEventProcessorApi::new()),
            Arc::new(MockCompanyChainEventProcessorApi::new()),
            Arc::new(MockIdentityChainEventProcessorApi::new()),
        );
        let event = get_test_identity_chain_event();

        let result = service.send_identity_chain_events(event).await;
        assert!(result.is_ok());
    }

    fn generate_test_event(
        previous: Option<nostr::event::Event>,
        root: Option<nostr::event::Event>,
        height: usize,
        block: BillBlock,
    ) -> nostr::event::Event {
        let keys = BcrKeys::from_private_key(&private_key_test());
        let block_event = Event::new_bill_chain(BillBlockEvent {
            bill_id: bill_id_test(),
            block: block.clone(),
            block_height: height,
        })
        .try_into()
        .expect("could not create envelope");
        create_public_chain_event(
            &bill_id_test().to_string(),
            block_event,
            Timestamp::new(1000).unwrap(),
            BlockchainType::Bill,
            previous,
            root,
        )
        .expect("could not create chain event")
        .finalize(&keys.get_nostr_keys())
        .expect("could not sign event")
    }

    #[tokio::test]
    async fn test_validate_bill_blocks_exist_on_nostr_chain_empty() {
        let mut transport = MockNotificationJsonTransport::new();
        transport.expect_resolve_public_chain().never();
        let service = get_service_with_transport(transport, MockNostrChainEventStore::new());
        let bill_id = bill_id_test();

        let res = service
            .validate_bill_blocks_exist_on_nostr_chain(&bill_id, &[])
            .await;
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn test_validate_bill_blocks_exist_on_nostr_chain_invalid() {
        let mut transport = MockNotificationJsonTransport::new();
        // create a block
        let block = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();
        transport
            .expect_resolve_public_chain()
            .times(1)
            .returning(move |_, _| Ok(vec![generate_test_event(None, None, 1, block.clone())]));
        let service = get_service_with_transport(transport, MockNostrChainEventStore::new());
        let bill_id = bill_id_test();

        // create a different block
        let block = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();

        let res = service
            .validate_bill_blocks_exist_on_nostr_chain(&bill_id, &[block])
            .await;
        assert!(!res.unwrap());
    }

    #[tokio::test]
    async fn test_validate_bill_blocks_exist_on_nostr_chain_valid() {
        // create a block
        let block = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();
        let mut transport = MockNotificationJsonTransport::new();
        let block_clone = block.clone();
        transport
            .expect_resolve_public_chain()
            .times(1)
            .returning(move |_, _| {
                Ok(vec![generate_test_event(
                    None,
                    None,
                    1,
                    block_clone.clone(),
                )])
            });
        let service = get_service_with_transport(transport, MockNostrChainEventStore::new());
        let bill_id = bill_id_test();

        let res = service
            .validate_bill_blocks_exist_on_nostr_chain(&bill_id, &[block])
            .await;
        assert!(res.unwrap());
    }

    #[tokio::test]
    async fn test_validate_bill_blocks_exist_on_nostr_chain_multi() {
        // create a block
        let block = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();
        // create another block
        let mut block_2 = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();
        block_2.id = BlockId::first().add(1);
        let mut transport = MockNotificationJsonTransport::new();
        let block_clone = block.clone();
        let block_2_clone = block_2.clone();
        transport
            .expect_resolve_public_chain()
            .times(1)
            .returning(move |_, _| {
                let first_event = generate_test_event(None, None, 1, block_clone.clone());
                let second_event = generate_test_event(
                    Some(first_event.clone()),
                    Some(first_event.clone()),
                    2,
                    block_2_clone.clone(),
                );
                Ok(vec![first_event, second_event])
            });
        let service = get_service_with_transport(transport, MockNostrChainEventStore::new());
        let bill_id = bill_id_test();

        // works if it's not sorted correctly as well
        let res = service
            .validate_bill_blocks_exist_on_nostr_chain(&bill_id, &[block_2, block])
            .await;
        assert!(res.unwrap());
    }
}
