use std::sync::Arc;

use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::event::{CompanyBlockEvent, Event, EventEnvelope};
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::blockchain::BlockchainType};
use bcr_ebill_persistence::{
    NostrChainEventStoreApi, company::CompanyStoreApi, nostr::NostrChainEvent,
};
use log::{debug, error, trace, warn};

use crate::{EventType, transport::root_and_reply_id};
use bcr_ebill_api::service::transport_service::Result;

use super::{CompanyChainEventProcessorApi, NotificationHandlerApi};

pub struct CompanyChainEventHandler {
    company_store: Arc<dyn CompanyStoreApi>,
    processor: Arc<dyn CompanyChainEventProcessorApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationHandlerApi for CompanyChainEventHandler {
    fn handles_event(&self, event_type: &EventType) -> bool {
        event_type == &EventType::CompanyChain
    }

    async fn handle_event(
        &self,
        event: EventEnvelope,
        _node_id: &NodeId, // this is only useful for DM based notifications
        _sender: Option<nostr::key::PublicKey>,
        original_event: Option<Box<nostr::event::Event>>,
    ) -> Result<()> {
        debug!("incoming company chain event");
        if let Ok(decoded) = Event::<CompanyBlockEvent>::try_from(event.clone()) {
            if let Ok(keys) = self.company_store.get_key_pair(&decoded.data.node_id).await {
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
                        original_event,
                        decoded.data.block_height,
                        &decoded.data.block.hash,
                        &decoded.data.node_id.to_string(),
                    )
                    .await?;
                }
            } else {
                trace!("no keys for incoming company block");
            }
        } else {
            warn!("Could not decode event to CompanyBlockEvent {event:?}");
        }
        Ok(())
    }
}

impl ServiceTraitBounds for CompanyChainEventHandler {}

impl CompanyChainEventHandler {
    pub fn new(
        company_store: Arc<dyn CompanyStoreApi>,
        processor: Arc<dyn CompanyChainEventProcessorApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    ) -> Self {
        Self {
            company_store,
            processor,
            chain_event_store,
        }
    }
    async fn store_event(
        &self,
        event: Box<nostr::event::Event>,
        block_height: usize,
        block_hash: &Sha256Hash,
        chain_id: &str,
    ) -> Result<()> {
        let (root, reply) = root_and_reply_id(&event);
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
                chain_type: BlockchainType::Company,
                block_height,
                block_hash: block_hash.to_owned(),
                received: Timestamp::now(),
                time: event.created_at.into(),
                payload: *event,
            })
            .await
        {
            error!("Failed to store company chain nostr event into event store {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use bcr_ebill_core::{
        application::company::Company,
        protocol::{
            Name,
            blockchain::{Blockchain, company::CompanyBlockchain},
            crypto::BcrKeys,
        },
    };
    use mockall::predicate::{always, eq};

    use crate::handler::{
        MockCompanyChainEventProcessorApi,
        company_chain_event_processor::tests::{
            get_company_create_block, get_company_update_block,
        },
        test_utils::{
            MockCompanyStore, MockNostrChainEventStore, get_company_data, get_test_nostr_event,
            node_id_test, private_key_test, update_company_block_with_name,
        },
    };

    use super::*;

    #[tokio::test]
    async fn test_handle_update_event() {
        let (mut store, mut processor, mut chain_event_store) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let chain = create_company_chain(node_id.clone(), company.clone(), &keys);
        let data = update_company_block_with_name(Some(Name::new("new_name").unwrap()));
        let block = get_company_update_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );
        let original_event = Box::new(get_test_nostr_event());

        let event = Event::new_company_chain(CompanyBlockEvent {
            node_id: node_id_test(),
            block_height: 1,
            block: block.clone(),
        });

        // see if we have chain keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
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

        let handler = CompanyChainEventHandler::new(
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

    #[tokio::test]
    async fn test_handle_no_chain_event() {
        let (mut store, mut processor, chain_event_store) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let chain = create_company_chain(node_id.clone(), company.clone(), &keys);
        let data = update_company_block_with_name(Some(Name::new("new_name").unwrap()));
        let block = get_company_update_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );
        let original_event = Box::new(get_test_nostr_event());

        let event = Event::new_company_chain(CompanyBlockEvent {
            node_id: node_id_test(),
            block_height: 1,
            block: block.clone(),
        });

        // no chain keys so we should be done here
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "company block".to_string(),
                    node_id_test().to_string(),
                ))
            })
            .once();

        processor.expect_process_chain_data().never();

        let handler = CompanyChainEventHandler::new(
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

    fn create_company_chain(
        node_id: NodeId,
        company: Company,
        keys: &BcrKeys,
    ) -> CompanyBlockchain {
        let blocks = vec![get_company_create_block(node_id, company, keys)];
        CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain")
    }

    fn create_mocks() -> (
        MockCompanyStore,
        MockCompanyChainEventProcessorApi,
        MockNostrChainEventStore,
    ) {
        (
            MockCompanyStore::default(),
            MockCompanyChainEventProcessorApi::default(),
            MockNostrChainEventStore::default(),
        )
    }
}
