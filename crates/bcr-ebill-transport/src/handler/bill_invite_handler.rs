use std::{str::FromStr, sync::Arc};

use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::{
        ProtocolValidationError,
        blockchain::BlockchainType,
        crypto::BcrKeys,
        event::{ChainInvite, Event, EventEnvelope},
    },
};
use bcr_ebill_persistence::NostrChainEventStoreApi;
use log::{debug, error, warn};

use crate::{
    EventType,
    handler::public_chain_helpers::{BlockData, EventContainer},
};
use bcr_ebill_api::service::transport_service::Result;

use super::{BillChainEventProcessorApi, NotificationHandlerApi};

#[derive(Clone)]
pub struct BillInviteEventHandler {
    processor: Arc<dyn BillChainEventProcessorApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationHandlerApi for BillInviteEventHandler {
    fn handles_event(&self, event_type: &EventType) -> bool {
        event_type == &EventType::BillChainInvite
    }

    async fn handle_event(
        &self,
        event: EventEnvelope,
        node_id: &NodeId,
        _sender: Option<nostr::key::PublicKey>,
        _: Option<Box<nostr::event::Event>>,
    ) -> Result<()> {
        debug!("incoming bill chain invite for {node_id}");
        if let Ok(decoded) = Event::<ChainInvite>::try_from(event.clone()) {
            let keys = BcrKeys::from_private_key(&decoded.data.keys.private_key);
            let chain_id =
                BillId::from_str(&decoded.data.chain_id).map_err(ProtocolValidationError::from)?;

            let mut inserted_chain: Vec<EventContainer> = Vec::new();
            if let Ok(chain_data) = self.processor.resolve_chain(&chain_id, &keys).await {
                // We try to add shorter and shorter chains until we have a success
                for data in chain_data.iter() {
                    let blocks = data
                        .iter()
                        .filter_map(|d| match d.block.clone() {
                            BlockData::Bill(block) => Some(block),
                            _ => None,
                        })
                        .collect();
                    if !data.is_empty()
                        && self
                            .processor
                            .process_chain_data(
                                &BillId::from_str(&decoded.data.chain_id)
                                    .map_err(ProtocolValidationError::from)?,
                                blocks,
                                Some(BcrKeys::from_private_key(&decoded.data.keys.private_key)),
                            )
                            .await
                            .is_ok()
                    {
                        inserted_chain = data.to_owned();
                        break;
                    }
                }
                // we are onboarded to the chain so store all Nostr chain data also the invalid one
                if let Err(e) = self
                    .store_events(
                        &decoded.data.chain_id,
                        decoded.data.chain_type,
                        inserted_chain,
                        &chain_data,
                    )
                    .await
                {
                    error!("Error storing chain events: {e}");
                }
            } else {
                error!("Could not extract chain data for invite event {event:?}");
            }
        } else {
            warn!("Could not decode event to ChainInvite {event:?}");
        }
        Ok(())
    }
}

impl ServiceTraitBounds for BillInviteEventHandler {}

impl BillInviteEventHandler {
    pub fn new(
        processor: Arc<dyn BillChainEventProcessorApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    ) -> Self {
        Self {
            processor,
            chain_event_store,
        }
    }

    async fn store_events(
        &self,
        chain_id: &str,
        chain_type: BlockchainType,
        inserted_chain: Vec<EventContainer>,
        _chains: &[Vec<EventContainer>],
    ) -> Result<()> {
        for (idx, inserted) in inserted_chain.iter().enumerate() {
            let data = inserted.as_chain_store_event(chain_id, chain_type, idx + 1);
            if let Err(e) = self.chain_event_store.add_chain_event(data).await {
                debug!("Could not store chain event because {e}")
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        handler::{
            MockBillChainEventProcessorApi,
            public_chain_helpers::collect_event_chains,
            test_utils::{
                MockNostrChainEventStore, bill_id_test, get_bill_keys, get_genesis_chain,
                node_id_test, private_key_test,
            },
        },
        transport::create_public_chain_event,
    };

    use super::*;
    use bcr_ebill_core::protocol::event::BillBlockEvent;
    use bcr_ebill_core::{
        protocol::Timestamp, protocol::blockchain::Blockchain, protocol::crypto::BcrKeys,
    };
    use mockall::predicate::{always, eq};
    use nostr::event::FinalizeEvent;

    #[test]
    fn test_single_block() {
        let (_keys, chain) = generate_test_chain(1, false);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );
        assert_eq!(chains.len(), 1, "should contain a single valid chain");
        let result_chain = chains.first().unwrap();
        assert_eq!(result_chain.len(), 1, "chain should contain a single event");
    }

    #[test]
    fn test_multiple_valid_blocks() {
        let (_keys, chain) = generate_test_chain(3, false);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );

        assert_eq!(chains.len(), 1, "should contain a single valid chain");
        let result_chain = chains.first().unwrap();
        assert_eq!(result_chain.len(), 3, "chain should contain 3 events");
    }

    #[test]
    fn test_multiple_chains() {
        let (_keys, chain) = generate_test_chain(3, true);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );

        assert_eq!(chains.len(), 2, "should contain two valid chains");
        for chain in chains {
            assert_eq!(chain.len(), 3, "chain should contain 3 events");
        }
    }

    #[tokio::test]
    async fn test_process_single_event_chain_invite() {
        let (mut processor, mut chain_event_store) = get_mocks();

        let node_id = node_id_test();
        let (_bcr_keys, chain) = generate_test_chain(1, false);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );

        // get events from nostr
        processor
            .expect_resolve_chain()
            .with(eq(bill_id_test()), always())
            .returning(move |_, _| Ok(chains.clone()));

        // process blocks
        processor
            .expect_process_chain_data()
            .withf(|bill_id, blocks, keys| {
                bill_id == &bill_id_test()
                    && blocks.len() == 1
                    && keys.clone().unwrap().pub_key().to_string()
                        == get_bill_keys().pub_key().to_string()
            })
            .returning(|_, _, _| Ok(()));

        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(1);

        let event = generate_test_event(&BcrKeys::new(), None, None, 1);
        let invite = Event::new_bill_invite(ChainInvite::bill(
            bill_id_test().to_string(),
            get_bill_keys(),
        ))
        .try_into()
        .expect("failed to create envelope");

        let handler = BillInviteEventHandler::new(Arc::new(processor), Arc::new(chain_event_store));
        handler
            .handle_event(invite, &node_id, None, Some(Box::new(event.clone())))
            .await
            .expect("failed to process chain invite event");
    }

    #[tokio::test]
    async fn test_process_single_chain_invite() {
        let (mut processor, mut chain_event_store) = get_mocks();

        let node_id = node_id_test();
        let (_bcr_keys, chain) = generate_test_chain(3, false);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );

        // get events from nostr
        processor
            .expect_resolve_chain()
            .with(eq(bill_id_test()), always())
            .returning(move |_, _| Ok(chains.clone()));

        // process blocks
        processor
            .expect_process_chain_data()
            .withf(|bill_id, blocks, keys| {
                bill_id == &bill_id_test()
                    && blocks.len() == 3
                    && keys.clone().unwrap().pub_key().to_string()
                        == get_bill_keys().pub_key().to_string()
            })
            .returning(|_, _, _| Ok(()));

        // store events
        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(3);

        let event = generate_test_event(&BcrKeys::new(), None, None, 1);
        let invite = Event::new_bill_invite(ChainInvite::bill(
            bill_id_test().to_string(),
            get_bill_keys(),
        ))
        .try_into()
        .expect("failed to create envelope");

        let handler = BillInviteEventHandler::new(Arc::new(processor), Arc::new(chain_event_store));
        handler
            .handle_event(invite, &node_id, None, Some(Box::new(event.clone())))
            .await
            .expect("failed to process chain invite event");
    }

    #[tokio::test]
    async fn test_process_multiple_chains_invite() {
        let (mut processor, mut chain_event_store) = get_mocks();

        let node_id = node_id_test();
        let (_bcr_keys, chain) = generate_test_chain(3, true);
        let chains = collect_event_chains(
            &chain,
            &bill_id_test().to_string(),
            BlockchainType::Bill,
            &None,
        );

        // get events from nostr
        processor
            .expect_resolve_chain()
            .with(eq(bill_id_test()), always())
            .returning(move |_, _| Ok(chains.clone()));

        // process blocks
        processor
            .expect_process_chain_data()
            .withf(|bill_id, blocks, keys| {
                bill_id == &bill_id_test()
                    && blocks.len() == 3
                    && keys.clone().unwrap().pub_key().to_string()
                        == get_bill_keys().pub_key().to_string()
            })
            .returning(|_, _, _| Ok(()));

        // store events
        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(3);

        let event = generate_test_event(&BcrKeys::new(), None, None, 1);
        let invite = Event::new_bill_invite(ChainInvite::bill(
            bill_id_test().to_string(),
            get_bill_keys(),
        ))
        .try_into()
        .expect("failed to create envelope");

        let handler = BillInviteEventHandler::new(Arc::new(processor), Arc::new(chain_event_store));
        handler
            .handle_event(invite, &node_id, None, Some(Box::new(event.clone())))
            .await
            .expect("failed to process chain invite event");
    }

    fn get_mocks() -> (MockBillChainEventProcessorApi, MockNostrChainEventStore) {
        (
            MockBillChainEventProcessorApi::new(),
            MockNostrChainEventStore::new(),
        )
    }

    // generates event chains. If invalid blocks is enabled chains of size 3 will have two equal
    // valid chains. From there on len even gives one valid and N - 2 invalid (shorter) chains.
    // Uneven give two valid (equal len) and N - 1 invalid chains.
    fn generate_test_chain(
        len: usize,
        invalid_blocks: bool,
    ) -> (BcrKeys, Vec<nostr::event::Event>) {
        let keys = BcrKeys::from_private_key(&private_key_test());
        let mut result = Vec::new();

        let root = generate_test_event(&keys, None, None, 1);
        result.push(root.clone());

        let mut parent = root.clone();
        for idx in 1..len {
            let child =
                generate_test_event(&keys, Some(parent.clone()), Some(root.clone()), idx + 1);
            result.push(child.clone());
            // produce some side chain
            if invalid_blocks && idx % 2 == 0 {
                let invalid =
                    generate_test_event(&keys, Some(parent.clone()), Some(root.clone()), idx + 1);
                result.push(invalid);
            }
            parent = child;
        }

        (keys, result)
    }

    #[allow(dead_code)]
    fn print_chains(chains: Vec<Vec<EventContainer>>) {
        for (idx, chain) in chains.iter().enumerate() {
            println!("CHAIN: {idx}");
            for (edx, evt) in chain.iter().enumerate() {
                println!(
                    "Evt {edx}: {:?} {:?} {:?} {}",
                    evt.root_id,
                    evt.event.id,
                    evt.reply_id,
                    evt.children.len()
                );
            }
        }
    }

    fn generate_test_event(
        keys: &BcrKeys,
        previous: Option<nostr::event::Event>,
        root: Option<nostr::event::Event>,
        height: usize,
    ) -> nostr::event::Event {
        create_public_chain_event(
            &bill_id_test().to_string(),
            generate_test_block(height),
            Timestamp::new(1000).unwrap(),
            BlockchainType::Bill,
            previous,
            root,
        )
        .expect("could not create chain event")
        .finalize(&keys.get_nostr_keys())
        .expect("could not sign event")
    }

    fn generate_test_block(block_height: usize) -> EventEnvelope {
        let block = get_genesis_chain(None)
            .blocks()
            .first()
            .expect("could not get block")
            .clone();

        Event::new_bill_chain(BillBlockEvent {
            bill_id: bill_id_test(),
            block: block.clone(),
            block_height,
        })
        .try_into()
        .expect("could not create envelope")
    }
}
