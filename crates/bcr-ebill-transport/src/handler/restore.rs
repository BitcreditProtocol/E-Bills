use std::sync::Arc;

use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_api::service::transport_service::{
    Result, restore::RestoreAccountApi, transport_client::TransportClientApi,
};
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::blockchain::{BlockchainType, identity::IdentityBlock},
    protocol::crypto::BcrKeys,
};
use log::{error, info};
use nostr::filter::Filter;

use crate::handler::DirectMessageEventProcessorApi;
use bcr_ebill_persistence::NostrChainEventStoreApi;

use super::{
    IdentityChainEventProcessorApi,
    public_chain_helpers::{BlockData, EventContainer, resolve_event_chains},
};

#[allow(dead_code)]
pub struct RestoreAccountService {
    nostr: Arc<dyn TransportClientApi>,
    identity_chain_processor: Arc<dyn IdentityChainEventProcessorApi>,
    dm_processor: Arc<dyn DirectMessageEventProcessorApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    keys: BcrKeys,
    node_id: NodeId,
}

#[allow(dead_code)]
impl RestoreAccountService {
    pub async fn new(
        nostr: Arc<dyn TransportClientApi>,
        identity_chain_processor: Arc<dyn IdentityChainEventProcessorApi>,
        dm_processor: Arc<dyn DirectMessageEventProcessorApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
        keys: BcrKeys,
        node_id: NodeId,
    ) -> Self {
        Self {
            nostr,
            identity_chain_processor,
            dm_processor,
            chain_event_store,
            keys,
            node_id,
        }
    }

    async fn restore_primary_account(&self) -> Result<()> {
        // restores identity and all our own companies
        self.restore_identity().await?;

        // restore bills and companies primary account was invited to
        self.process_private_events().await?;
        Ok(())
    }

    async fn process_private_events(&self) -> Result<()> {
        let events = self
            .nostr
            .resolve_private_events(Filter::new().since(nostr::types::Timestamp::zero()))
            .await?;
        info!("found private {} dms for primary account", events.len());
        for event in events {
            if let Err(e) = self
                .dm_processor
                .process_direct_message(Box::new(event))
                .await
            {
                error!("Failed to process direct message with: {e}");
            }
        }
        Ok(())
    }

    async fn restore_identity(&self) -> Result<()> {
        info!("restoring identity chain");
        let chains = resolve_event_chains(
            self.nostr.clone(),
            &self.node_id.to_string(),
            BlockchainType::Identity,
            &Some(self.keys.to_owned()),
        )
        .await?;
        info!("found {} chains for primary account", chains.len());
        for chain in chains {
            if self.valid_identity_chain_events(&chain) {
                let blocks: Vec<IdentityBlock> = chain
                    .iter()
                    .filter_map(|d| match d.block.clone() {
                        BlockData::Identity(block) => Some(block),
                        _ => None,
                    })
                    .collect();
                self.identity_chain_processor
                    .process_chain_data(&self.node_id, blocks, Some(self.keys.clone()))
                    .await?;
                for event_container in &chain {
                    let event = event_container.as_chain_store_event(
                        &self.node_id.to_string(),
                        BlockchainType::Identity,
                        event_container.block.get_block_height() as usize,
                    );
                    if let Err(e) = self.chain_event_store.add_chain_event(event).await {
                        error!("Failed to store identity chain event during restore: {e}");
                    }
                }
                info!("restored identity chain from {} events", chain.len());
            }
        }
        Ok(())
    }

    fn valid_identity_chain_events(&self, events: &[EventContainer]) -> bool {
        let mut valid = true;
        for event in events {
            valid = valid
                && self
                    .identity_chain_processor
                    .validate_chain_event_and_sender(&self.node_id, event.event.pubkey);
        }
        valid
    }
}

impl ServiceTraitBounds for RestoreAccountService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl RestoreAccountApi for RestoreAccountService {
    async fn restore_account(&self) -> Result<()> {
        info!("restoring primary account");
        self.restore_primary_account().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bcr_ebill_core::{
        application::identity::Identity,
        protocol::Timestamp,
        protocol::blockchain::{
            Blockchain,
            identity::{IdentityBlockchain, IdentityCreateBlockData},
        },
        protocol::event::{Event, EventEnvelope, IdentityBlockEvent},
    };
    use mockall::predicate::{always, eq};
    use nostr::event::FinalizeEvent;

    use crate::{
        handler::{
            MockDirectMessageEventProcessorApi, MockIdentityChainEventProcessorApi,
            test_utils::{MockNostrChainEventStore, get_baseline_identity, private_key_test},
        },
        test_utils::{MockNotificationJsonTransport, node_id_test, test_ts},
        transport::create_public_chain_event,
    };

    use super::*;

    #[tokio::test]
    async fn test_service() {
        let mut nostr = MockNotificationJsonTransport::new();
        let processor = MockIdentityChainEventProcessorApi::new();
        let dm_processor = MockDirectMessageEventProcessorApi::new();
        let chain_event_store = MockNostrChainEventStore::new();
        let keys = BcrKeys::new();

        nostr
            .expect_resolve_public_chain()
            .returning(|_, _| Ok(vec![]))
            .once();

        nostr
            .expect_resolve_private_events()
            .returning(|_| Ok(vec![]))
            .once();

        let service = RestoreAccountService::new(
            Arc::new(nostr),
            Arc::new(processor),
            Arc::new(dm_processor),
            Arc::new(chain_event_store),
            keys,
            node_id_test(),
        )
        .await;

        service
            .restore_account()
            .await
            .expect("could not restore account");
    }

    #[tokio::test]
    async fn test_create_identity() {
        let (keys, events) = generate_test_chain(1, false);
        let mut nostr = MockNotificationJsonTransport::new();
        let mut processor = MockIdentityChainEventProcessorApi::new();
        let dm_processor = MockDirectMessageEventProcessorApi::new();
        let mut chain_event_store = MockNostrChainEventStore::new();

        let _return_events = events.clone();
        nostr
            .expect_resolve_public_chain()
            .returning(move |_, _| Ok(_return_events.clone()))
            .once();

        // should validate the event sender
        processor
            .expect_validate_chain_event_and_sender()
            .with(eq(node_id_test()), eq(keys.get_nostr_keys().public_key()))
            .returning(|_, _| true)
            .times(events.len());

        processor
            .expect_process_chain_data()
            .with(eq(node_id_test()), always(), eq(Some(keys.clone())))
            .returning(|_, _, _| Ok(()))
            .once();

        chain_event_store
            .expect_add_chain_event()
            .with(always())
            .returning(|_| Ok(()))
            .times(events.len());

        nostr
            .expect_resolve_private_events()
            .returning(|_| Ok(vec![]))
            .once();

        let service = RestoreAccountService::new(
            Arc::new(nostr),
            Arc::new(processor),
            Arc::new(dm_processor),
            Arc::new(chain_event_store),
            keys,
            node_id_test(),
        )
        .await;

        service
            .restore_account()
            .await
            .expect("could not restore account");
    }

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

    fn generate_test_event(
        keys: &BcrKeys,
        previous: Option<nostr::event::Event>,
        root: Option<nostr::event::Event>,
        height: usize,
    ) -> nostr::event::Event {
        create_public_chain_event(
            &node_id_test().to_string(),
            generate_test_block(height),
            Timestamp::new(1000).unwrap(),
            BlockchainType::Identity,
            previous,
            root,
        )
        .expect("could not create chain event")
        .finalize(&keys.get_nostr_keys())
        .expect("could not sign event")
    }

    fn generate_test_block(block_height: usize) -> EventEnvelope {
        let identity = get_baseline_identity();
        let block = get_valid_identity_chain(&identity.identity, &identity.key_pair)
            .get_latest_block()
            .clone();

        Event::new_identity_chain(IdentityBlockEvent {
            node_id: identity.identity.node_id.clone(),
            block,
            block_height,
        })
        .try_into()
        .expect("could not create envelope")
    }

    pub fn get_valid_identity_chain(identity: &Identity, keys: &BcrKeys) -> IdentityBlockchain {
        IdentityBlockchain::new(
            &IdentityCreateBlockData::from(identity.to_owned()),
            keys,
            test_ts(),
        )
        .unwrap()
    }
}
