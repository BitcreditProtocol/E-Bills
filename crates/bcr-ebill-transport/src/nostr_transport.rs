use std::collections::HashMap;
use std::sync::Arc;

use bcr_common::core::NodeId;
use bcr_ebill_api::service::transport_service::transport_client::TransportClientApi;
use bcr_ebill_api::util::validate_node_id_network;
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::application::nostr_contact::TrustLevel;
use bcr_ebill_core::protocol::Address;
use bcr_ebill_core::protocol::City;
use bcr_ebill_core::protocol::Country;
use bcr_ebill_core::protocol::Name;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::blockchain::BlockchainType;
use bcr_ebill_core::protocol::blockchain::bill::{
    block::ContactType,
    participant::{BillAnonParticipant, BillIdentParticipant, BillParticipant},
};
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bcr_ebill_core::protocol::event::{BillChainEventPayload, Event, EventEnvelope};
use bcr_ebill_persistence::ContactStoreApi;
use bcr_ebill_persistence::nostr::{
    NostrChainEvent, NostrChainEventStoreApi, NostrContactStoreApi, NostrQueuedMessage,
    NostrQueuedMessageStoreApi,
};
use bitcoin::base58;
use log::info;
use log::{debug, error, warn};
use tokio_with_wasm::alias as tokio;

use bcr_ebill_api::service::transport_service::{Error, Result};
use bcr_ebill_core::protocol::PostalAddress;

#[derive(borsh::BorshDeserialize)]
struct LegacyBillEventMessage {
    event_type: bcr_ebill_core::protocol::event::EventType,
    version: String,
    data: BillChainEventPayload,
}

/// Transport implementation for Nostr
#[derive(Clone)]
pub struct NostrTransportService {
    nostr_client: Arc<dyn TransportClientApi>,
    contact_store: Arc<dyn ContactStoreApi>,
    nostr_contact_store: Arc<dyn NostrContactStoreApi>,
    queued_message_store: Arc<dyn NostrQueuedMessageStoreApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    nostr_relays: Vec<url::Url>,
}

impl ServiceTraitBounds for NostrTransportService {}

impl NostrTransportService {
    // the number of times we want to retry sending a block message
    const NOSTR_MAX_RETRIES: i32 = 10;

    pub fn new(
        nostr_client: Arc<dyn TransportClientApi>,
        contact_store: Arc<dyn ContactStoreApi>,
        nostr_contact_store: Arc<dyn NostrContactStoreApi>,
        queued_message_store: Arc<dyn NostrQueuedMessageStoreApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
        nostr_relays: Vec<url::Url>,
    ) -> Self {
        Self {
            nostr_client,
            contact_store,
            nostr_contact_store,
            queued_message_store,
            chain_event_store,
            nostr_relays,
        }
    }

    pub(crate) fn get_node_transport(&self, _node_id: &NodeId) -> Arc<dyn TransportClientApi> {
        // With single shared client, we return it for any node_id
        // The client internally handles multi-identity
        self.nostr_client.clone()
    }

    pub(crate) fn get_first_transport(&self) -> Arc<dyn TransportClientApi> {
        // With single client, just return it
        self.nostr_client.clone()
    }

    pub(crate) fn get_local_identity(&self, node_id: &NodeId) -> Option<BillParticipant> {
        // Check if this node_id is one of the local identities managed by the client.
        // Only return Some if the node_id exists in the signers map.
        if self.nostr_client.has_local_signer(node_id) {
            Some(BillParticipant::Ident(BillIdentParticipant {
                // we create an ident, but it doesn't matter, since we just need the node id and nostr relay
                t: ContactType::Person,
                node_id: node_id.to_owned(),
                email: None,
                name: Name::new("default name").expect("is a valid name"),
                postal_address: PostalAddress {
                    country: Country::AT,
                    city: City::new("default city").expect("is valid city"),
                    zip: None,
                    address: Address::new("default address").expect("is valid address"),
                },
                nostr_relays: self.nostr_relays.clone(),
            }))
        } else {
            None
        }
    }

    pub(crate) async fn resolve_identity(&self, node_id: &NodeId) -> Option<BillParticipant> {
        match self.get_local_identity(node_id) {
            Some(id) => Some(id),
            None => {
                if let Some(identity) = self.resolve_node_contact(node_id).await {
                    Some(identity)
                } else if let Ok(Some(nostr)) = self.nostr_contact_store.by_node_id(node_id).await
                    && nostr.trust_level != TrustLevel::None
                {
                    // we have no contact but a nostr contact of a participant
                    Some(BillParticipant::Anon(BillAnonParticipant {
                        node_id: node_id.to_owned(),
                        nostr_relays: nostr.relays,
                    }))
                } else {
                    None
                }
            }
        }
    }

    pub(crate) async fn resolve_node_contact(&self, node_id: &NodeId) -> Option<BillParticipant> {
        if validate_node_id_network(node_id).is_err() {
            return None;
        }
        if let Ok(Some(identity)) = self.contact_store.get(node_id).await {
            identity.try_into().ok()
        } else {
            None
        }
    }

    pub(crate) async fn add_identity(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<()> {
        debug!("Adding identity for node_id: {node_id}");
        self.nostr_client
            .add_identity(node_id.clone(), keys.clone())
            .await?;
        Ok(())
    }

    pub(crate) async fn send_all_bill_events(
        &self,
        sender: &NodeId,
        events: &HashMap<NodeId, Event<BillChainEventPayload>>,
    ) -> Result<()> {
        let node = self.get_node_transport(sender);
        for (node_id, event_to_process) in events.iter() {
            if let Some(identity) = self.resolve_identity(node_id).await {
                let message: EventEnvelope = event_to_process.clone().try_into()?;
                if let Err(e) = node
                    .send_private_event(sender, &identity, message.clone())
                    .await
                {
                    error!("Failed to send block notification, will add it to retry queue: {e}");
                    self.queue_retry_message_and_trigger(
                        sender,
                        Some(node_id),
                        base58::encode(&borsh::to_vec(&message)?),
                    )
                    .await?;
                }
            } else {
                warn!("Failed to find recipient in contacts for node_id: {node_id}");
            }
        }
        Ok(())
    }

    pub(crate) async fn send_private_event(
        &self,
        sender: &NodeId,
        recipient: &NodeId,
        relays: &[url::Url],
        message: EventEnvelope,
    ) -> Result<()> {
        let transport = self.get_node_transport(sender);
        let recipient = BillParticipant::Anon(BillAnonParticipant {
            node_id: recipient.to_owned(),
            nostr_relays: relays.to_vec(),
        });
        transport
            .send_private_event(sender, &recipient, message)
            .await?;
        Ok(())
    }

    pub(crate) async fn queue_retry_message(
        &self,
        sender: &NodeId,
        recipient: Option<&NodeId>,
        payload: String,
    ) -> Result<()> {
        let queue_message = NostrQueuedMessage {
            id: uuid::Uuid::new_v4().to_string(),
            sender_id: sender.to_owned(),
            recipient: recipient.map(|n| n.to_owned()),
            payload,
        };
        self.queued_message_store
            .add_message(queue_message, Self::NOSTR_MAX_RETRIES)
            .await
            .map_err(|e| {
                error!("Failed to add send nostr event to retry queue: {e}");
                Error::Persistence(format!(
                    "Failed to add send nostr event to retry queue: {e}"
                ))
            })?;
        Ok(())
    }

    pub(crate) async fn queue_retry_message_and_trigger(
        &self,
        sender: &NodeId,
        recipient: Option<&NodeId>,
        payload: String,
    ) -> Result<()> {
        self.queue_retry_message(sender, recipient, payload).await?;

        let retry_target = recipient
            .map(ToString::to_string)
            .unwrap_or_else(|| "public broadcast".to_string());
        debug!(
            "Queued Nostr retry message for sender {sender}; triggering immediate retry for {retry_target}"
        );

        let retry_service = self.clone();
        tokio::spawn(async move {
            if let Err(e) = retry_service.send_retry_messages().await {
                error!("Failed to process Nostr retry queue after enqueue: {e}");
            }
        });

        Ok(())
    }

    pub(crate) async fn find_root_and_previous_event(
        &self,
        previous_hash: &Sha256Hash,
        chain_id: &str,
        chain_type: BlockchainType,
    ) -> Result<(Option<NostrChainEvent>, Option<NostrChainEvent>)> {
        // find potential previous block event
        let previous_event = self
            .chain_event_store
            .find_by_block_hash(previous_hash)
            .await
            .map_err(|_| Error::Persistence("failed to read from chain events".to_owned()))?;

        // if there is a previous and it is not the root event, also get the root event
        let root_event = if previous_event.clone().is_some_and(|f| !f.is_root_event()) {
            self.chain_event_store
                .find_root_event(chain_id, chain_type)
                .await
                .map_err(|_| Error::Persistence("failed to read from chain events".to_owned()))?
        } else {
            previous_event.clone()
        };
        Ok((previous_event, root_event))
    }

    // sends all required bill chain events like public bill data and bill invites
    pub(crate) async fn add_chain_event(
        &self,
        event: &nostr::event::Event,
        root: &Option<NostrChainEvent>,
        previous: &Option<NostrChainEvent>,
        chain_id: &str,
        chain_type: BlockchainType,
        block_height: usize,
        block_hash: &Sha256Hash,
    ) -> Result<()> {
        self.chain_event_store
            .add_chain_event(NostrChainEvent {
                event_id: event.id.to_string(),
                root_id: root
                    .clone()
                    .map(|e| e.event_id.to_string())
                    .unwrap_or(event.id.to_string()),
                reply_id: previous.clone().map(|e| e.event_id.to_string()),
                author: event.pubkey.to_string(),
                chain_id: chain_id.to_string(),
                chain_type,
                block_height,
                block_hash: block_hash.to_owned(),
                received: event.created_at.into(),
                time: event.created_at.into(),
                payload: event.clone(),
            })
            .await
            .map_err(|_| Error::Persistence("failed to write to chain events".to_owned()))?;
        Ok(())
    }

    pub(crate) async fn send_retry_messages(&self) -> Result<()> {
        let mut failed_ids = vec![];
        while let Ok(Some(queued_message)) = self
            .queued_message_store
            .get_retry_messages(1)
            .await
            .map(|r| r.first().cloned())
        {
            let result = match &queued_message.recipient {
                Some(node_id) => {
                    // Private message retry: payload is base58-encoded borsh EventEnvelope
                    let decoded = match base58::decode(&queued_message.payload) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            error!("Failed to decode base58 private retry payload: {e}");
                            failed_ids.push(queued_message.id.clone());
                            continue;
                        }
                    };
                    let message = match borsh::from_slice::<EventEnvelope>(&decoded) {
                        Ok(message) => Ok(message),
                        Err(envelope_err) => {
                            match borsh::from_slice::<LegacyBillEventMessage>(&decoded) {
                                Ok(legacy_event) => match borsh::to_vec(&legacy_event.data) {
                                    Ok(data) => Ok(EventEnvelope {
                                        event_type: legacy_event.event_type,
                                        version: legacy_event.version,
                                        data,
                                    }),
                                    Err(e) => Err(Error::Message(e.to_string())),
                                },
                                Err(legacy_err) => {
                                    error!(
                                        "Failed to deserialize private retry payload as envelope ({envelope_err}) or legacy event ({legacy_err})"
                                    );
                                    Err(Error::Message(envelope_err.to_string()))
                                }
                            }
                        }
                    };

                    match message {
                        Ok(message) => {
                            self.send_retry_private_message(
                                &queued_message.sender_id,
                                node_id,
                                message,
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    }
                }
                None => {
                    // Public broadcast retry: payload is JSON nostr::event::Event
                    match serde_json::from_str::<nostr::event::Event>(&queued_message.payload) {
                        Ok(event) => {
                            let node = self.get_node_transport(&queued_message.sender_id);
                            node.broadcast_event(&event).await
                        }
                        Err(e) => {
                            error!("Failed to deserialize public retry payload: {e}");
                            Err(Error::Message(e.to_string()))
                        }
                    }
                }
            };

            match result {
                Ok(()) => {
                    info!("Successfully sent retry message {}", queued_message.id);
                    if let Err(e) = self
                        .queued_message_store
                        .succeed_retry(&queued_message.id)
                        .await
                    {
                        error!("Failed to mark retry message as sent: {e}");
                    }
                }
                Err(e) => {
                    error!("Failed to send retry message: {e}");
                    failed_ids.push(queued_message.id.clone());
                }
            }
        }

        for failed in failed_ids {
            if let Err(e) = self.queued_message_store.fail_retry(&failed).await {
                error!("Failed to store failed retry attemt: {e}");
            }
        }
        Ok(())
    }

    async fn send_retry_private_message(
        &self,
        sender: &NodeId,
        node_id: &NodeId,
        message: EventEnvelope,
    ) -> Result<()> {
        let node = self.get_node_transport(sender);
        if let Some(identity) = self.resolve_identity(node_id).await {
            node.send_private_event(sender, &identity, message).await?;
        } else {
            warn!("Failed to resolve recipient for retry message, node_id: {node_id}");
            return Err(Error::Message(format!(
                "Could not resolve recipient for retry message: {node_id}"
            )));
        }
        Ok(())
    }

    pub(crate) async fn connect(&self) {
        // With single multi-identity client, just connect it
        if let Err(e) = self.nostr_client.connect().await {
            error!("Failed to connect to transport: {e}");
        }
    }
}
