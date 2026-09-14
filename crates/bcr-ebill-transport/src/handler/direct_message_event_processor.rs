use std::sync::Arc;

use crate::{
    NostrClient, Result,
    chain_keys::ChainKeyServiceApi,
    handler::{FileMetadataProcessorApi, NotificationHandlerApi},
    nostr::{add_offset, determine_recipient, process_event, should_process},
};
use async_trait::async_trait;
use bcr_ebill_api::service::contact_service::ContactServiceApi;
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_persistence::NostrEventOffsetStoreApi;

use crate::handler::DirectMessageEventProcessorApi;

#[derive(Clone)]
pub struct DirectMessageEventProcessor {
    client: Arc<NostrClient>,
    contact_service: Arc<dyn ContactServiceApi>,
    offset_store: Arc<dyn NostrEventOffsetStoreApi>,
    chain_key_service: Arc<dyn ChainKeyServiceApi>,
    handlers: Vec<Arc<dyn NotificationHandlerApi>>,
    file_metadata_processor: Arc<dyn FileMetadataProcessorApi>,
}

impl DirectMessageEventProcessor {
    pub async fn new(
        client: Arc<NostrClient>,
        contact_service: Arc<dyn ContactServiceApi>,
        offset_store: Arc<dyn NostrEventOffsetStoreApi>,
        chain_key_service: Arc<dyn ChainKeyServiceApi>,
        handlers: Vec<Arc<dyn NotificationHandlerApi>>,
        file_metadata_processor: Arc<dyn FileMetadataProcessorApi>,
    ) -> Self {
        Self {
            client,
            contact_service,
            offset_store,
            chain_key_service,
            handlers,
            file_metadata_processor,
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl DirectMessageEventProcessorApi for DirectMessageEventProcessor {
    async fn process_direct_message(&self, event: Box<nostr::event::Event>) -> Result<()> {
        // Get all local node IDs for multi-identity handling
        let local_node_ids = self.client.get_all_node_ids();

        // Check if the event should be processed
        if should_process(
            event.clone(),
            &local_node_ids,
            &self.contact_service,
            &self.offset_store,
            None,
        )
        .await
        {
            // Determine which identity should receive this event
            match determine_recipient(&event, &self.client).await {
                Ok((recipient_node_id, signer)) => {
                    // Process event with the correct identity's signer
                    let (success, time) = process_event(
                        event.clone(),
                        signer,
                        recipient_node_id.clone(),
                        self.chain_key_service.clone(),
                        &self.handlers,
                        self.file_metadata_processor.clone(),
                    )
                    .await?;

                    // Add event offset for the recipient identity
                    add_offset(
                        &self.offset_store,
                        event.id,
                        time,
                        success,
                        &recipient_node_id,
                    )
                    .await;
                }
                Err(e) => {
                    log::debug!("Could not determine recipient for event {}: {e}", event.id);
                }
            }
        }
        Ok(())
    }
}

impl ServiceTraitBounds for DirectMessageEventProcessor {}
