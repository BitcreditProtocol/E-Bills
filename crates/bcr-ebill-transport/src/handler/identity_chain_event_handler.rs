use std::sync::Arc;

use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::event::{Event, EventEnvelope, IdentityBlockEvent};
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::blockchain::BlockchainType};
use bcr_ebill_persistence::identity::IdentityStoreApi;
use bcr_ebill_persistence::{NostrChainEventStoreApi, nostr::NostrChainEvent};
use log::{debug, error, trace, warn};

use crate::{EventType, transport::root_and_reply_id};
use bcr_ebill_api::service::transport_service::Result;

use super::{IdentityChainEventProcessorApi, NotificationHandlerApi};

#[derive(Clone)]
pub struct IdentityChainEventHandler {
    identity_store: Arc<dyn IdentityStoreApi>,
    processor: Arc<dyn IdentityChainEventProcessorApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationHandlerApi for IdentityChainEventHandler {
    fn handles_event(&self, event_type: &EventType) -> bool {
        event_type == &EventType::IdentityChain
    }

    async fn handle_event(
        &self,
        event: EventEnvelope,
        _node_id: &NodeId, // this is only useful for DM based notifications
        _sender: Option<nostr::key::PublicKey>,
        original_event: Option<Box<nostr::event::Event>>,
    ) -> Result<()> {
        debug!("incoming identity chain event");
        if let Ok(decoded) = Event::<IdentityBlockEvent>::try_from(event.clone()) {
            if let Ok(keys) = self.identity_store.get_key_pair().await {
                let valid = self
                    .processor
                    .process_chain_data(
                        &decoded.data.node_id,
                        vec![decoded.data.block.clone()],
                        Some(keys.clone()),
                    )
                    .await
                    .inspect_err(|e| error!("Received invalid block {e}"))
                    .is_ok();

                if valid && let Some(original_event) = original_event {
                    self.store_event(
                        &original_event,
                        decoded.data.block_height,
                        &decoded.data.block.hash,
                        &decoded.data.node_id.to_string(),
                    )
                    .await?;
                }
            } else {
                trace!("no keys for incoming identity block");
            }
        } else {
            warn!("Could not decode event to IdentityBlockEvent {event:?}");
        }
        Ok(())
    }
}
impl ServiceTraitBounds for IdentityChainEventHandler {}

#[allow(unused)]
impl IdentityChainEventHandler {
    pub fn new(
        identity_store: Arc<dyn IdentityStoreApi>,
        processor: Arc<dyn IdentityChainEventProcessorApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    ) -> Self {
        Self {
            identity_store,
            processor,
            chain_event_store,
        }
    }
    async fn store_event(
        &self,
        event: &nostr::event::Event,
        block_height: usize,
        block_hash: &Sha256Hash,
        chain_id: &str,
    ) -> Result<()> {
        let (root, reply) = root_and_reply_id(event);
        if let Err(e) = self
            .chain_event_store
            .add_chain_event(NostrChainEvent {
                event_id: event.id.to_string(),
                root_id: root
                    .map(|id| id.to_string())
                    .unwrap_or(event.id.to_string()),
                reply_id: reply.map(|id| id.to_string()),
                author: event.pubkey.to_string(),
                chain_id: chain_id.to_string(),
                chain_type: BlockchainType::Identity,
                block_height,
                block_hash: block_hash.to_owned(),
                received: Timestamp::now(),
                time: event.created_at.into(),
                payload: event.clone(),
            })
            .await
        {
            error!("Failed to store identity chain nostr event into event store {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use bcr_ebill_core::{
        application::identity::IdentityWithAll,
        protocol::Name,
        protocol::blockchain::{
            Blockchain,
            identity::{IdentityBlock, IdentityBlockchain},
        },
    };
    use mockall::predicate::always;

    use crate::{
        handler::{
            MockIdentityChainEventProcessorApi,
            identity_chain_event_processor::tests::{
                get_identity_create_block, get_identity_update_block,
            },
            test_utils::{
                MockIdentityStore, MockNostrChainEventStore, get_baseline_identity,
                get_test_nostr_event, node_id_test, update_identity_block_with_name,
            },
        },
        test_utils::test_ts,
    };

    use super::*;

    #[tokio::test]
    async fn test_handle_update_event() {
        let (mut store, mut processor, mut chain_event_store) = create_mocks();
        let full = get_baseline_identity();
        let identity = full.identity.clone();
        let keys = full.key_pair.clone();
        let chain = create_identity_chain(full.clone());
        let data = update_identity_block_with_name(Some(Name::new("new_name").unwrap()));
        let block = get_identity_update_block(chain.get_latest_block(), &keys, &data);
        let original_event = Box::new(get_test_nostr_event());

        let event = Event::new_company_chain(IdentityBlockEvent {
            node_id: node_id_test(),
            block_height: 1,
            block: block.clone(),
        });

        // see if we have chain keys
        store
            .expect_get_key_pair()
            .returning(move || Ok(keys.clone()))
            .once();

        // should process the chain data
        processor
            .expect_process_chain_data()
            .withf(|node_id, blocks, company_keys| {
                node_id == &node_id.clone() && !blocks.is_empty() && company_keys.is_some()
            })
            .returning(|_, _, _| Ok(()))
            .once();

        // and store the nostr event
        chain_event_store
            .expect_add_chain_event()
            .with(always())
            .returning(|_| Ok(()))
            .once();

        let handler = IdentityChainEventHandler::new(
            Arc::new(store),
            Arc::new(processor),
            Arc::new(chain_event_store),
        );

        handler
            .handle_event(
                event.try_into().unwrap(),
                &identity.node_id,
                None,
                Some(original_event),
            )
            .await
            .expect("failed to handle event");
    }

    #[tokio::test]
    async fn test_handle_no_chain_event() {
        let (mut store, mut processor, chain_event_store) = create_mocks();
        let full = get_baseline_identity();
        let identity = full.identity.clone();
        let keys = full.key_pair.clone();
        let node_id = identity.node_id.clone();
        let chain = create_identity_chain(full.clone());
        let data = update_identity_block_with_name(Some(Name::new("new_name").unwrap()));
        let block = IdentityBlock::create_block_for_update(
            chain.get_latest_block(),
            &data,
            &keys,
            test_ts(),
        )
        .expect("could not create block");

        let original_event = Box::new(get_test_nostr_event());

        let event = Event::new_identity_chain(IdentityBlockEvent {
            node_id: node_id.clone(),
            block_height: 1,
            block: block.clone(),
        });

        // no chain keys so we should be done here
        store
            .expect_get_key_pair()
            .returning(move || {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "identity key pair".to_string(),
                    "".to_string(),
                ))
            })
            .once();

        processor.expect_process_chain_data().never();

        let handler = IdentityChainEventHandler::new(
            Arc::new(store),
            Arc::new(processor),
            Arc::new(chain_event_store),
        );

        handler
            .handle_event(
                event.try_into().unwrap(),
                &node_id,
                None,
                Some(original_event),
            )
            .await
            .expect("failed to handle event");
    }

    fn create_identity_chain(full: IdentityWithAll) -> IdentityBlockchain {
        let blocks = vec![get_identity_create_block(full.identity, &full.key_pair)];
        IdentityBlockchain::new_from_blocks(blocks).expect("could not create chain")
    }

    fn create_mocks() -> (
        MockIdentityStore,
        MockIdentityChainEventProcessorApi,
        MockNostrChainEventStore,
    ) {
        (
            MockIdentityStore::default(),
            MockIdentityChainEventProcessorApi::default(),
            MockNostrChainEventStore::default(),
        )
    }
}
