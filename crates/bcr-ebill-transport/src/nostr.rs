use crate::{
    chain_keys::ChainKeyServiceApi,
    handler::{FileMetadataProcessorApi, NotificationHandlerApi},
    transport::{
        chain_filter, create_public_chain_event, decrypt_or_decode_public_chain_event,
        unwrap_direct_message, unwrap_public_chain_event,
    },
};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::{
    application::nostr_contact::{NostrContact, TrustLevel},
    protocol::{
        Timestamp,
        blockchain::{BlockchainType, bill::participant::BillParticipant},
        crypto::BcrKeys,
    },
};
use bitcoin::base58;
use log::{debug, error, info, trace, warn};
use nostr::{
    event::{Event, EventBuilder, EventId, FinalizeEvent, Kind, Tag},
    filter::{Filter, SingleLetterTag},
    key::{Keys, PublicKey},
    nips::{
        nip01::Metadata,
        nip17::PrivateDirectMessageBuilder,
        nip19::ToBech32,
        nip65::{RelayList, RelayMetadata, extract_relay_list},
    },
    types::url::RelayUrl,
};
use nostr_sdk::{
    authenticator::SignerAuthenticator,
    client::{Client, ClientNotification, ReqTarget, SendEventOutput},
    relay::RelayStatus,
};
use std::sync::{Arc, Mutex, atomic::Ordering};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::AtomicBool,
    time::Duration,
};

use bcr_ebill_api::{
    constants::NOSTR_MAX_RELAYS,
    service::{
        contact_service::ContactServiceApi,
        file_server_service::resolve_blossom_servers,
        transport_service::{
            Error, NostrConfig, NostrContactData, Result, transport_client::TransportClientApi,
        },
    },
};
use bcr_ebill_core::{application::ServiceTraitBounds, protocol::event::EventEnvelope};
use bcr_ebill_persistence::{NostrEventOffset, NostrEventOffsetStoreApi, NostrStoreApi};
use futures::{
    StreamExt,
    channel::mpsc::{UnboundedReceiver, unbounded},
};

use tokio::task::JoinSet;
use tokio_with_wasm::alias as tokio;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

const BLOSSOM_SERVER_LIST_KIND: Kind = Kind::Custom(10063);
const FILE_METADATA_KIND: Kind = Kind::Custom(1063);
const DM_BACKFILL_LIMIT: usize = 1000;

/// Check the output of sending an event to Nostr relays.
/// Logs warnings for individual relay failures and returns an error if no relay
/// accepted the event.
fn check_send_output(output: SendEventOutput, context: &str) -> Result<()> {
    for (relay, error) in &output.failed {
        warn!("{context}: relay {relay} failed: {error}");
    }
    if output.success.is_empty() {
        error!("{context}: all relays failed to accept the event");
        return Err(Error::Network(format!(
            "{context}: all relays failed to accept the event"
        )));
    }
    Ok(())
}

/// A wrapper around nostr_sdk that implements the NotificationJsonTransportApi.
///
/// # Example:
/// ```no_run
/// let config = NostrConfig::new(
///     BcrKeys::new(),
///     vec!["wss://relay.example.com".to_string()],
///     "My Company".to_string(),
/// );
/// let transport = NostrClient::new(&config).await.unwrap();
/// transport.send(&recipient, event).await.unwrap();
/// ```
/// We use the latest GiftWrap and PrivateDirectMessage already with this if I
/// understand the nostr-sdk docs and sources correctly.
/// @see https://nips.nostr.com/59 and https://nips.nostr.com/17
#[derive(Clone)]
pub struct NostrClient {
    client: Client,
    signers: Arc<Mutex<HashMap<NodeId, Arc<Keys>>>>,
    relays: Vec<url::Url>,
    blossom_servers: Vec<url::Url>,
    default_timeout: Duration,
    connected: Arc<AtomicBool>,
    sync_running: Arc<AtomicBool>, // Prevents concurrent sync
    max_relays: Option<usize>,
    relay_ack_threshold: usize,
    nostr_contact_store: Option<Arc<dyn NostrStoreApi>>, // Keep for backwards compatibility
    nostr_store: Option<Arc<dyn NostrStoreApi>>,
}

impl NostrClient {
    /// Creates a new nostr client with multiple identities sharing a relay pool
    pub async fn new(
        identities: Vec<(NodeId, BcrKeys)>,
        relays: Vec<url::Url>,
        blossom_servers: Vec<url::Url>,
        default_timeout: Duration,
        max_relays: Option<usize>,
        relay_ack_threshold: usize,
        nostr_store: Option<Arc<dyn NostrStoreApi>>,
    ) -> Result<Self> {
        if identities.is_empty() {
            return Err(Error::Message("At least one identity required".to_string()));
        }
        // Use first identity to construct the underlying Client
        let first_keys = &identities[0].1;
        let client = Client::builder()
            .authenticator(SignerAuthenticator::new(
                first_keys.get_nostr_keys().clone(),
            ))
            .build();

        // Add all relays to the shared pool
        for relay in &relays {
            client.add_relay(relay.to_string()).await.map_err(|e| {
                error!("Failed to add relay to Nostr client: {e}");
                Error::Network("Failed to add relay to Nostr client".to_string())
            })?;
        }

        // Build signers HashMap from all identities
        let mut signers = HashMap::new();
        for (node_id, keys) in identities {
            signers.insert(node_id, Arc::new(keys.get_nostr_keys()));
        }

        Ok(Self {
            client,
            signers: Arc::new(Mutex::new(signers)),
            relays,
            blossom_servers,
            default_timeout,
            connected: Arc::new(AtomicBool::new(false)),
            sync_running: Arc::new(AtomicBool::new(false)),
            max_relays,
            relay_ack_threshold: relay_ack_threshold.max(1),
            nostr_contact_store: nostr_store.clone(), // Keep for backwards compatibility
            nostr_store,
        })
    }

    /// Creates a new nostr client with the given config.
    pub async fn default(config: &NostrConfig) -> Result<Self> {
        let identities = vec![(config.node_id.clone(), config.keys.clone())];
        Self::new(
            identities,
            config.relays.clone(),
            config.blossom_servers.clone(),
            config.default_timeout,
            None, // max_relays not available in old config
            config.relay_ack_threshold,
            None, // store not available
        )
        .await
    }

    /// Get the signer for a specific identity
    pub fn get_signer(&self, node_id: &NodeId) -> Result<Arc<Keys>> {
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(node_id)
            .cloned()
            .ok_or_else(|| Error::Message(format!("No signer found for node_id: {}", node_id)))
    }

    /// Add a new identity to this client
    pub fn add_identity(&self, node_id: NodeId, keys: BcrKeys) -> Result<()> {
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(node_id, Arc::new(keys.get_nostr_keys()));
        Ok(())
    }

    /// Check if this client has a local signer for the given node_id
    pub fn has_local_signer(&self, node_id: &NodeId) -> bool {
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(node_id)
    }

    /// Get all node_ids managed by this client
    pub fn get_all_node_ids(&self) -> Vec<NodeId> {
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Subscribe to some nostr events with a filter
    pub async fn subscribe(&self, subscription: Filter) -> Result<()> {
        self.client()
            .await?
            .subscribe(subscription)
            .await
            .map_err(|e| {
                error!("Failed to subscribe to Nostr events: {e}");
                Error::Network("Failed to subscribe to Nostr events".to_string())
            })?;
        Ok(())
    }

    /// Returns the latest metadata event for the given npub either from the provided relays or
    /// from this clients relays.
    pub async fn fetch_metadata(&self, npub: PublicKey) -> Result<Option<Metadata>> {
        let filter = Filter::new().author(npub).kind(Kind::Metadata);

        let events = self
            .client()
            .await?
            .fetch_events(filter)
            .timeout(self.default_timeout)
            .await
            .map_err(|e| {
                error!("Failed to fetch Nostr metadata: {e}");
                Error::Network("Failed to fetch Nostr metadata".to_string())
            })?;

        let Some(event) = events.iter().max_by_key(|e| e.created_at) else {
            return Ok(None);
        };

        let metadata = Metadata::try_from(event).map_err(|e| {
            error!("Failed to decode Nostr metadata: {e}");
            Error::Message("Failed to decode Nostr metadata".to_string())
        })?;

        Ok(Some(metadata))
    }

    /// Returns the relays a given npub is reading from or writing to.
    // Relay list content (the actual relay urls) are stored as tags on the event. The event
    // content itself is actually empty. Here we look for tags with a lowercase 'r' (specified
    // as RelayMetadata) and filter for valid ones. Filter standardized filters and parses the
    // matching tags into enum values.
    pub async fn fetch_relay_list(
        &self,
        npub: PublicKey,
        relays: Vec<url::Url>,
    ) -> Result<Vec<RelayUrl>> {
        let filter = Filter::new().author(npub).kind(Kind::RelayList).limit(1);
        let events = self.fetch_events(filter, None, Some(relays)).await?;
        Ok(events
            .first()
            .map(|e| {
                extract_relay_list(e)
                    .map(|(relay_url, _metadata)| relay_url)
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn fetch_blossom_server_list(
        &self,
        npub: PublicKey,
        relays: Vec<url::Url>,
    ) -> Result<Vec<url::Url>> {
        let filter = Filter::new()
            .author(npub)
            .kind(BLOSSOM_SERVER_LIST_KIND)
            .limit(1);
        let events = self.fetch_events(filter, None, Some(relays)).await?;
        Ok(events
            .first()
            .map(|event| {
                event
                    .tags
                    .iter()
                    .filter_map(|tag| match tag.as_slice() {
                        [kind, url, ..] if kind == "server" => url::Url::parse(url).ok(),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Returns events that match filter from either the provided relays or from this clients
    /// relays. If a order is provided, the events are sorted accordingly otherwise the default
    /// descending order is used.
    pub async fn fetch_events(
        &self,
        filter: Filter,
        order: Option<SortOrder>,
        relays: Option<Vec<url::Url>>,
    ) -> Result<Vec<Event>> {
        let relays = match relays {
            Some(relays) => relays,
            None => self.relays.clone(),
        };

        let target = ReqTarget::manual(
            to_relay_urls(&relays)?
                .into_iter()
                .map(|relay| (relay, vec![filter.clone()])),
        );

        let events = self
            .client()
            .await?
            .fetch_events(target)
            .timeout(self.default_timeout)
            .await
            .map_err(|e| {
                error!("Failed to fetch Nostr events: {e}");
                Error::Network("Failed to fetch Nostr events".to_string())
            })?;
        let mut events = events.into_iter().collect::<Vec<Event>>();
        if Some(SortOrder::Asc) == order {
            events.reverse();
        }
        Ok(events)
    }

    /// Stream events from specific relays using the provided filter.
    /// Returns a stream that yields events as they arrive from relays.
    /// This is more efficient than fetch_events for large result sets as it doesn't
    /// wait for all relays to respond before returning results.
    pub async fn stream_events_from(
        &self,
        filter: Filter,
        relays: Option<Vec<url::Url>>,
        timeout: Option<Duration>,
    ) -> Result<impl futures::Stream<Item = Event>> {
        let relays = match relays {
            Some(relays) => relays,
            None => self.relays.clone(),
        };

        let target = ReqTarget::manual(
            to_relay_urls(&relays)?
                .into_iter()
                .map(|relay| (relay, vec![filter.clone()])),
        );

        let stream = self
            .client()
            .await?
            .stream_events(target)
            .timeout(timeout.unwrap_or(self.default_timeout))
            .await
            .map_err(|e| {
                error!("Failed to stream Nostr events: {e}");
                Error::Network("Failed to stream Nostr events".to_string())
            })?;

        Ok(stream.filter_map(|(relay, result)| async move {
            match result {
                Ok(event) => Some(event),
                Err(e) => {
                    debug!("Error streaming event from {relay}: {e}");
                    None
                }
            }
        }))
    }

    /// Send an event to specific relays
    pub async fn send_event_to(&self, relays: Vec<url::Url>, event: &Event) -> Result<()> {
        let output = self
            .client()
            .await?
            .send_event(event)
            .to(to_relay_urls(&relays)?)
            .await
            .map_err(|e| {
                error!("Failed to send event to relays: {e}");
                Error::Network(format!("Failed to send event to relays: {e}"))
            })?;
        check_send_output(output, "send_event_to")
    }

    /// Get the default timeout for this client
    pub fn get_default_timeout(&self) -> Duration {
        self.default_timeout
    }

    async fn send_nip17_message(
        &self,
        sender_node_id: &NodeId,
        recipient: &BillParticipant,
        event: EventEnvelope,
    ) -> Result<()> {
        // Get the Keys for the specified sender identity
        let sender_keys = self.get_signer(sender_node_id)?;

        let receiver_pubkey = recipient.node_id().npub();
        let message = base58::encode(&borsh::to_vec(&event)?);

        let event = PrivateDirectMessageBuilder::new(receiver_pubkey, message)
            .finalize(sender_keys.as_ref())
            .map_err(|e| {
                error!("Failed to create NIP-17 event: {e}");
                Error::Message(format!("Failed to create NIP-17 event: {e}"))
            })?;

        let relays = recipient.nostr_relays();
        let output = if !relays.is_empty() {
            self.client()
                .await?
                .send_event(&event)
                .to(to_relay_urls(&relays)?)
                .await
                .map_err(|e| {
                    error!("Error sending NIP-17 message to specific relays: {e}");
                    Error::Network(format!("Failed to send NIP-17 message: {e}"))
                })?
        } else {
            self.client().await?.send_event(&event).await.map_err(|e| {
                error!("Error sending NIP-17 message: {e}");
                Error::Network(format!("Failed to send NIP-17 message: {e}"))
            })?
        };
        check_send_output(output, "send_nip17_message")?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub async fn client(&self) -> Result<&Client> {
        if !self.is_connected() {
            self.connect().await?;
        }
        Ok(&self.client)
    }

    /// Calculate the complete relay set from user relays + contact relays
    async fn calculate_relay_set(&self) -> Result<HashSet<url::Url>> {
        // Get contacts from store if available
        let contacts = if let Some(store) = &self.nostr_contact_store {
            store.get_all().await.map_err(|e| {
                error!("Failed to fetch contacts for relay calculation: {e}");
                Error::Message("Failed to fetch contacts".to_string())
            })?
        } else {
            vec![]
        };

        Ok(calculate_relay_set_internal(
            &self.relays,
            &contacts,
            self.max_relays,
        ))
    }

    /// Update the client's relay connections to match the target set
    async fn update_relays(&self, target_relays: HashSet<url::Url>) -> Result<()> {
        let client = &self.client;

        // Get current relays
        let current_relays: HashSet<url::Url> = client
            .relays()
            .await
            .keys()
            .map(|url| url.to_owned().into())
            .collect();

        // Add new relays
        for relay in target_relays.iter() {
            if !current_relays.contains(relay) {
                match client.add_relay(relay.to_string()).await {
                    Ok(_) => debug!("Added relay: {}", relay),
                    Err(e) => warn!("Failed to add relay {}: {}", relay, e),
                }
            }
        }

        // Remove old relays (relays not in target set)
        for relay in current_relays.iter() {
            if !target_relays.contains(relay) {
                // Convert url::Url to RelayUrl
                if let Ok(relay_url) = relay.as_str().parse::<RelayUrl>() {
                    match client.remove_relay(relay_url).await {
                        Ok(_) => debug!("Removed relay: {}", relay),
                        Err(e) => warn!("Failed to remove relay {}: {}", relay, e),
                    }
                }
            }
        }

        Ok(())
    }

    /// Public method to refresh relay connections based on current contacts
    pub async fn refresh_relays(&self) -> Result<()> {
        info!("Refreshing relay connections based on contacts");
        let relay_set = self.calculate_relay_set().await?;
        self.update_relays(relay_set).await?;
        info!(
            "Relay refresh complete, connected to {} relays",
            self.client.relays().await.len()
        );
        Ok(())
    }

    pub async fn has_connected_relays(&self) -> bool {
        self.client
            .relays()
            .await
            .values()
            .any(|relay| relay.status() == RelayStatus::Connected)
    }
}

impl ServiceTraitBounds for NostrClient {}

async fn queue_failed_relay_sync(store: &Arc<dyn NostrStoreApi>, relay: &RelayUrl, event: &Event) {
    let relay_url = match url::Url::parse(relay.as_str()) {
        Ok(url) => url,
        Err(parse_err) => {
            error!("Failed to parse relay url {relay}: {parse_err}");
            return;
        }
    };
    if let Err(store_err) = store.add_failed_relay_sync(&relay_url, event.clone()).await {
        error!("Failed to queue failed relay sync for {relay}: {store_err}");
    }
}

async fn collect_remaining_broadcast_results(
    mut rx: UnboundedReceiver<(RelayUrl, std::result::Result<EventId, Error>)>,
    mut success: HashSet<RelayUrl>,
    nostr_store: Option<Arc<dyn NostrStoreApi>>,
    event: Event,
) {
    while let Some((relay, result)) = rx.next().await {
        match result {
            Ok(_) => {
                success.insert(relay);
            }
            Err(e) => {
                if let Some(store) = &nostr_store {
                    queue_failed_relay_sync(store, &relay, &event).await;
                }
                debug!("Optimistic broadcast background failure for {relay}: {e}");
            }
        }
    }
    debug!(
        "Optimistic broadcast background completion: {} succeeded",
        success.len()
    );
}

fn to_relay_url(url: &url::Url) -> Result<RelayUrl> {
    RelayUrl::parse(url.as_str())
        .map_err(|e| Error::Message(format!("Invalid Nostr relay URL {url}: {e}")))
}

fn to_relay_urls(urls: &[url::Url]) -> Result<Vec<RelayUrl>> {
    urls.iter().map(to_relay_url).collect()
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl TransportClientApi for NostrClient {
    async fn send_private_event(
        &self,
        sender_node_id: &NodeId,
        recipient: &BillParticipant,
        event: EventEnvelope,
    ) -> Result<()> {
        self.send_nip17_message(sender_node_id, recipient, event)
            .await?;
        Ok(())
    }

    async fn build_public_chain_event(
        &self,
        sender_node_id: &NodeId,
        id: &str,
        blockchain: BlockchainType,
        block_time: Timestamp,
        event: EventEnvelope,
        previous_event: Option<Event>,
        root_event: Option<Event>,
    ) -> Result<Event> {
        // Get the keys for this identity to sign with
        let signing_keys = self.get_signer(sender_node_id)?;

        let event_builder = create_public_chain_event(
            id,
            event,
            block_time,
            blockchain,
            previous_event,
            root_event,
        )?;

        event_builder.finalize(signing_keys.as_ref()).map_err(|e| {
            error!("Failed to sign Nostr event: {e}");
            Error::Crypto("Failed to sign Nostr event".to_string())
        })
    }

    async fn broadcast_event(&self, event: &Event) -> Result<()> {
        let output = self.client().await?.send_event(event).await.map_err(|e| {
            error!("Failed to send Nostr event: {e}");
            Error::Network("Failed to send Nostr event".to_string())
        })?;
        check_send_output(output, "broadcast_event")?;
        Ok(())
    }

    async fn broadcast_event_optimistic(&self, event: &Event, min_acks: usize) -> Result<()> {
        let client = self.client().await?;
        let event = event.clone();

        let relays: Vec<RelayUrl> = client
            .relays()
            .await
            .into_values()
            .filter(|relay| relay.capabilities().has_write())
            .map(|relay| relay.url().clone())
            .collect();

        if relays.is_empty() {
            return Err(Error::Network(
                "No write relays available for optimistic broadcast".to_string(),
            ));
        }

        let threshold = min_acks.max(1).min(relays.len());
        let nostr_store = self.nostr_store.clone();

        let (tx, mut rx) = unbounded::<(RelayUrl, std::result::Result<EventId, Error>)>();

        for relay in relays {
            let client = client.clone();
            let event = event.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = client.send_event(&event).to(vec![relay.clone()]).await;
                let mapped = match result {
                    Ok(output) => {
                        if output.success.contains_key(&relay) {
                            Ok(output.value)
                        } else {
                            let err = output
                                .failed
                                .get(&relay)
                                .cloned()
                                .unwrap_or_else(|| "unknown relay error".to_string());
                            Err(Error::Network(format!("relay {relay} failed: {err}")))
                        }
                    }
                    Err(e) => Err(Error::Network(format!("relay {relay} error: {e}"))),
                };
                let _ = tx.unbounded_send((relay, mapped));
            });
        }
        drop(tx);

        let mut success = HashSet::new();

        while let Some((relay, result)) = rx.next().await {
            match result {
                Ok(_) => {
                    success.insert(relay);
                    if success.len() >= threshold {
                        tokio::spawn(collect_remaining_broadcast_results(
                            rx,
                            success,
                            nostr_store,
                            event,
                        ));
                        return Ok(());
                    }
                }
                Err(_) => {
                    if let Some(store) = &nostr_store {
                        queue_failed_relay_sync(store, &relay, &event).await;
                    }
                }
            }
        }

        Err(Error::Network(format!(
            "broadcast_event_optimistic: only {} of {} required acks received",
            success.len(),
            threshold,
        )))
    }

    fn relay_ack_threshold(&self) -> usize {
        self.relay_ack_threshold
    }

    async fn resolve_contact(&self, node_id: &NodeId) -> Result<Option<NostrContactData>> {
        match self.fetch_metadata(node_id.npub()).await? {
            Some(meta) => {
                let relays = self
                    .fetch_relay_list(node_id.npub(), self.relays.clone())
                    .await?;
                let blossom_servers = self
                    .fetch_blossom_server_list(node_id.npub(), self.relays.clone())
                    .await?;
                Ok(Some(NostrContactData {
                    metadata: meta,
                    relays,
                    blossom_servers,
                }))
            }
            _ => Ok(None),
        }
    }

    async fn resolve_public_chain(
        &self,
        id: &str,
        chain_type: BlockchainType,
    ) -> Result<Vec<nostr::event::Event>> {
        let filter = chain_filter(id, chain_type);
        let events = self
            .fetch_events(filter, Some(SortOrder::Asc), None)
            .await?;
        Ok(events)
    }

    async fn add_contact_subscription(&self, node_id: &NodeId) -> Result<()> {
        debug!("adding nostr subscription for contact {node_id}");
        self.subscribe(Filter::new().author(node_id.npub())).await?;
        Ok(())
    }

    async fn resolve_private_events(&self, filter: Filter) -> Result<Vec<nostr::event::Event>> {
        let kinds = vec![Kind::GiftWrap];
        // Subscribe with all public keys from all identities
        let pubkeys: Vec<PublicKey> = self
            .signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .map(|node_id| node_id.npub())
            .collect();
        let filter = filter.clone().pubkeys(pubkeys).kinds(kinds);
        Ok(self
            .fetch_events(filter, Some(SortOrder::Asc), None)
            .await?)
    }

    async fn resolve_events(&self, filter: Filter) -> Result<Vec<nostr::event::Event>> {
        Ok(self
            .fetch_events(filter, Some(SortOrder::Asc), None)
            .await?)
    }

    async fn try_decrypt_private_event(
        &self,
        event: &nostr::event::Event,
    ) -> Result<Option<(NodeId, EventEnvelope, nostr::key::PublicKey)>> {
        match event.kind {
            Kind::GiftWrap => {
                let keys_to_try = prioritized_signers_for_event(event, &self.signers);
                for (node_id, nostr_keys) in keys_to_try {
                    if let Some((envelope, sender, _, _)) =
                        unwrap_direct_message(event, &nostr_keys).await
                    {
                        return Ok(Some((node_id, envelope, sender)));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    async fn publish_metadata(&self, node_id: &NodeId, data: &Metadata) -> Result<()> {
        // Get the signer for this identity
        let signer = self.get_signer(node_id)?;

        // Build and sign the metadata event with the specific identity
        let event = data.clone().finalize(signer.as_ref()).map_err(|e| {
            error!("Failed to sign metadata event: {e}");
            Error::Crypto("Failed to sign metadata event".to_string())
        })?;

        let output = self.client().await?.send_event(&event).await.map_err(|e| {
            error!("Failed to send user metadata with Nostr client: {e}");
            Error::Network("Failed to send user metadata with Nostr client".to_string())
        })?;
        check_send_output(output, "publish_metadata")
    }

    async fn publish_relay_list(&self, node_id: &NodeId, relays: Vec<RelayUrl>) -> Result<()> {
        // Get the signer for this identity
        let signer = self.get_signer(node_id)?;

        // Build and sign the relay list event with the specific identity
        let relay_list: Vec<(RelayUrl, Option<RelayMetadata>)> =
            relays.into_iter().map(|r| (r, None)).collect();
        let event = RelayList::new(relay_list)
            .finalize(signer.as_ref())
            .map_err(|e| {
                error!("Failed to sign relay list event: {e}");
                Error::Crypto("Failed to sign relay list event".to_string())
            })?;

        let output = self.client().await?.send_event(&event).await.map_err(|e| {
            error!("Failed to send relay list with Nostr client: {e}");
            Error::Network("Failed to send relay list with Nostr client".to_string())
        })?;
        check_send_output(output, "publish_relay_list")
    }

    async fn publish_blossom_server_list(
        &self,
        node_id: &NodeId,
        blossom_servers: Vec<url::Url>,
    ) -> Result<()> {
        let signer = self.get_signer(node_id)?;
        let tags = blossom_servers
            .into_iter()
            .map(|server| Tag::parse(["server", server.as_str()]))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| {
                error!("Failed to build Blossom server list tags: {e}");
                Error::Message("Failed to build Blossom server list tags".to_string())
            })?;
        let event = EventBuilder::new(BLOSSOM_SERVER_LIST_KIND, "")
            .tags(tags)
            .finalize(&*signer)
            .map_err(|e| {
                error!("Failed to sign Blossom server list event: {e}");
                Error::Crypto("Failed to sign Blossom server list event".to_string())
            })?;

        let output = self.client().await?.send_event(&event).await.map_err(|e| {
            error!("Failed to send Blossom server list with Nostr client: {e}");
            Error::Network("Failed to send Blossom server list with Nostr client".to_string())
        })?;
        check_send_output(output, "publish_blossom_server_list")
    }

    async fn publish_file_metadata(
        &self,
        node_id: &NodeId,
        plaintext_hash: &str,
        encrypted_hash: &str,
        server_urls: Vec<url::Url>,
        mime_type: Option<String>,
    ) -> Result<()> {
        if server_urls.is_empty() {
            debug!("Skipping file metadata publish - no server URLs available");
            return Ok(());
        }

        let signer = self.get_signer(node_id)?;

        // Build tags according to NIP-94 adapted conventions:
        // ox = plaintext local hash
        // x = encrypted blob hash
        // url = primary server URL
        // fallback = additional mirror URLs
        // m = MIME type when known
        let mut tags: Vec<Tag> = vec![
            Tag::parse(["ox", plaintext_hash]).map_err(|e| {
                error!("Failed to build ox tag: {e}");
                Error::Message("Failed to build ox tag".to_string())
            })?,
            Tag::parse(["x", encrypted_hash]).map_err(|e| {
                error!("Failed to build x tag: {e}");
                Error::Message("Failed to build x tag".to_string())
            })?,
        ];

        // Add primary URL
        if let Some(primary) = server_urls.first() {
            tags.push(Tag::parse(["url", primary.as_str()]).map_err(|e| {
                error!("Failed to build url tag: {e}");
                Error::Message("Failed to build url tag".to_string())
            })?);
        }

        // Add fallback URLs for additional mirrors
        for url in server_urls.iter().skip(1) {
            tags.push(Tag::parse(["fallback", url.as_str()]).map_err(|e| {
                error!("Failed to build fallback tag: {e}");
                Error::Message("Failed to build fallback tag".to_string())
            })?);
        }

        // Add MIME type if provided
        if let Some(ref mime) = mime_type {
            tags.push(Tag::parse(["m", mime]).map_err(|e| {
                error!("Failed to build m tag: {e}");
                Error::Message("Failed to build m tag".to_string())
            })?);
        }

        let event = EventBuilder::new(FILE_METADATA_KIND, "")
            .tags(tags)
            .finalize(&*signer)
            .map_err(|e| {
                error!("Failed to sign file metadata event: {e}");
                Error::Crypto("Failed to sign file metadata event".to_string())
            })?;

        let output = self.client().await?.send_event(&event).await.map_err(|e| {
            error!("Failed to send file metadata with Nostr client: {e}");
            Error::Network("Failed to send file metadata with Nostr client".to_string())
        })?;
        check_send_output(output, "publish_file_metadata")
    }

    async fn connect(&self) -> Result<()> {
        if self
            .connected
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.client.connect().await;

            // Publish relay list for ALL identities
            let node_ids = self.get_all_node_ids();
            let relay_urls: Vec<RelayUrl> = self
                .relays
                .iter()
                .filter_map(|r| RelayUrl::parse(r.as_str()).ok())
                .collect();
            let blossom_servers = resolve_blossom_servers(&self.blossom_servers, &self.relays);

            for node_id in node_ids {
                if let Err(e) = self.publish_relay_list(&node_id, relay_urls.clone()).await {
                    error!(
                        "Failed to publish relay list for identity {}: {}",
                        node_id, e
                    );
                }
                if let Err(e) = self
                    .publish_blossom_server_list(&node_id, blossom_servers.clone())
                    .await
                {
                    error!(
                        "Failed to publish Blossom server list for identity {}: {}",
                        node_id, e
                    );
                }
            }
        }
        Ok(())
    }

    async fn add_identity(&self, node_id: NodeId, keys: BcrKeys) -> Result<()> {
        // Add the identity to signers
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(node_id.clone(), Arc::new(keys.get_nostr_keys()));

        // Subscribe to direct messages for this identity if connected
        if self.is_connected() {
            let kinds = vec![Kind::GiftWrap];
            debug!("Adding subscription for direct messages to identity: {node_id}");
            self.subscribe(
                Filter::new()
                    .pubkey(node_id.npub())
                    .kinds(kinds)
                    .limit(DM_BACKFILL_LIMIT),
            )
            .await?;
            debug!("Adding subscription for public blocks messages from identity: {node_id}");
            self.subscribe(Filter::new().author(node_id.npub()).kind(Kind::TextNote))
                .await?;

            let relay_urls: Vec<RelayUrl> = self
                .relays
                .iter()
                .filter_map(|r| RelayUrl::parse(r.as_str()).ok())
                .collect();
            self.publish_relay_list(&node_id, relay_urls).await?;
            self.publish_blossom_server_list(
                &node_id,
                resolve_blossom_servers(&self.blossom_servers, &self.relays),
            )
            .await?;
        }

        Ok(())
    }

    fn has_local_signer(&self, node_id: &NodeId) -> bool {
        self.signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(node_id)
    }

    async fn sync_relays(&self) -> Result<()> {
        // Check if already running
        if self
            .sync_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            debug!("Relay sync already in progress, skipping");
            return Ok(());
        }

        // Perform sync (always release lock after)
        let result = async {
            if let Some(nostr_store) = &self.nostr_store {
                // Update last_seen_in_config for all user relays
                for relay in &self.relays {
                    nostr_store
                        .update_relay_last_seen(relay, Timestamp::now())
                        .await
                        .map_err(|e| {
                            Error::Message(format!("Failed to update relay last seen: {}", e))
                        })?;
                }

                // Run sync
                crate::relay_sync::sync_pending_relays(self, &self.relays, nostr_store).await?;
            }
            Ok(())
        }
        .await;

        // Always release the lock
        self.sync_running.store(false, Ordering::SeqCst);

        result
    }

    async fn retry_failed_syncs(&self) -> Result<()> {
        use bcr_ebill_api::constants::{RELAY_SYNC_MAX_RETRIES, RELAY_SYNC_RETRY_BATCH_SIZE};

        if let Some(nostr_store) = &self.nostr_store {
            for relay in &self.relays {
                let failed_events = nostr_store
                    .get_pending_relay_retries(relay, RELAY_SYNC_RETRY_BATCH_SIZE)
                    .await
                    .map_err(|e| Error::Message(format!("Failed to get retry events: {}", e)))?;

                for event in failed_events {
                    match self.send_event_to(vec![relay.clone()], &event).await {
                        Ok(_) => {
                            nostr_store
                                .mark_relay_retry_success(relay, &event.id.to_string())
                                .await
                                .map_err(|e| {
                                    Error::Message(format!("Failed to mark retry success: {}", e))
                                })?;
                        }
                        Err(e) => {
                            warn!("Retry failed for event {} to {}: {}", event.id, relay, e);
                            nostr_store
                                .mark_relay_retry_failed(
                                    relay,
                                    &event.id.to_string(),
                                    RELAY_SYNC_MAX_RETRIES,
                                )
                                .await
                                .map_err(|e| {
                                    Error::Message(format!("Failed to mark retry failure: {}", e))
                                })?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn query_file_metadata_events(
        &self,
        _file_hash: &str,
        nostr_hash: &str,
    ) -> Result<Vec<Event>> {
        let filter = Filter::new()
            .kind(FILE_METADATA_KIND)
            .custom_tag(SingleLetterTag::LOWERCASE_X, nostr_hash)
            .limit(50);

        self.fetch_events(filter, Some(SortOrder::Desc), None).await
    }
}

#[derive(Clone)]
pub struct NostrConsumer {
    client: Arc<NostrClient>,
    event_handlers: Vec<Arc<dyn NotificationHandlerApi>>,
    contact_service: Arc<dyn ContactServiceApi>,
    offset_store: Arc<dyn NostrEventOffsetStoreApi>,
    chain_key_service: Arc<dyn ChainKeyServiceApi>,
    file_metadata_processor: Arc<dyn FileMetadataProcessorApi>,
}

impl NostrConsumer {
    pub fn new(
        client: Arc<NostrClient>,
        contact_service: Arc<dyn ContactServiceApi>,
        event_handlers: Vec<Arc<dyn NotificationHandlerApi>>,
        offset_store: Arc<dyn NostrEventOffsetStoreApi>,
        chain_key_service: Arc<dyn ChainKeyServiceApi>,
        file_metadata_processor: Arc<dyn FileMetadataProcessorApi>,
    ) -> Self {
        Self {
            client,
            #[allow(clippy::arc_with_non_send_sync)]
            event_handlers,
            contact_service,
            chain_key_service,
            offset_store,
            file_metadata_processor,
        }
    }

    pub async fn start(&self) -> Result<JoinSet<()>> {
        // move dependencies into thread scope
        let client = self.client.clone();
        let event_handlers = self.event_handlers.clone();
        let contact_service = self.contact_service.clone();
        let offset_store = self.offset_store.clone();
        let chain_key_store = self.chain_key_service.clone();
        let file_metadata_processor = self.file_metadata_processor.clone();

        let mut tasks = JoinSet::new();

        // Get all local node IDs from the single client
        let local_node_ids: Vec<NodeId> = client
            .signers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();

        // Connect the client if not already connected
        if !client.is_connected()
            && let Err(e) = client.connect().await
        {
            error!("Failed to connect Nostr client: {e}");
        }

        // Get the earliest offset timestamp across all identities
        let mut earliest_offset = Timestamp::now();
        if !local_node_ids.is_empty() {
            for node_id in &local_node_ids {
                let offset = get_offset(&offset_store, node_id).await;
                if offset != Timestamp::zero() && offset < earliest_offset {
                    earliest_offset = offset;
                }
            }
        }

        // Subscribe to private events for ALL local identities (single subscription)
        let local_pubkeys: Vec<PublicKey> = local_node_ids.iter().map(|n| n.npub()).collect();

        client
            .subscribe(
                Filter::new()
                    .pubkeys(local_pubkeys.clone())
                    .kinds(vec![Kind::GiftWrap])
                    .limit(DM_BACKFILL_LIMIT),
            )
            .await
            .map_err(|e| {
                error!("Failed to subscribe to Nostr dm events: {e}");
                Error::Network("Failed to subscribe to Nostr dm events".to_string())
            })?;

        // Subscribe to public events from contacts and local identities
        let mut contacts = contact_service.get_nostr_npubs().await.unwrap_or_default();
        info!("Found {} contacts to subscribe to", contacts.len());
        contacts.append(&mut local_pubkeys.clone());
        info!("Subscribing to public Nostr events");

        client
            .subscribe(
                Filter::new()
                    .authors(contacts)
                    .kinds(vec![
                        Kind::TextNote,
                        Kind::RelayList,
                        Kind::Metadata,
                        BLOSSOM_SERVER_LIST_KIND,
                        FILE_METADATA_KIND,
                    ])
                    .since(earliest_offset.into()),
            )
            .await
            .map_err(|e| {
                error!("Failed to subscribe to Nostr public events: {e}");
                Error::Network("Failed to subscribe to Nostr public events".to_string())
            })?;

        // Spawn a SINGLE task for the single client
        let client_for_task = client.clone();
        tasks.spawn(async move {
            let result: Result<()> = async {
                let mut notifications = client_for_task.client.notifications();

                while let Some(note) = notifications.next().await {
                    let event_handlers = event_handlers.clone();
                    let offset_store = offset_store.clone();
                    let chain_key_store = chain_key_store.clone();
                    let contact_service = contact_service.clone();
                    let file_metadata_processor = file_metadata_processor.clone();
                    let local_node_ids = local_node_ids.clone();
                    let client = client.clone();

                    let ClientNotification::Event { event, .. } = note else {
                        continue;
                    };

                    if !should_process(
                        event.clone(),
                        &local_node_ids,
                        &contact_service,
                        &offset_store,
                        Some(earliest_offset),
                    )
                    .await
                    {
                        continue;
                    }

                    // Determine which local identity should receive this event
                    match determine_recipient(&event, &client).await {
                        Ok((recipient_node_id, signer)) => {
                            let (success, time) = process_event(
                                event.clone(),
                                signer,
                                recipient_node_id.clone(),
                                chain_key_store,
                                &event_handlers,
                                file_metadata_processor,
                            )
                            .await?;
                            // store the new event offset for the recipient identity
                            add_offset(&offset_store, event.id, time, success, &recipient_node_id)
                                .await;
                        }
                        Err(e) => {
                            debug!("Could not determine recipient for event {}: {e}", event.id);
                        }
                    }
                }

                Ok(())
            }
            .await;

            if let Err(e) = result {
                error!("Nostr notification handler failed: {e}");
            }
        });

        Ok(tasks)
    }
}

/// Determines which local identity should receive this event.
/// For private messages: tries to decrypt with each signer, returns the one that succeeds.
/// For public chain events: the recipient is determined by chain key ownership (all identities have access).
/// Returns the NodeId of the recipient identity and its signer.
pub async fn determine_recipient(
    event: &Event,
    client: &NostrClient,
) -> Result<(NodeId, Arc<Keys>)> {
    match event.kind {
        Kind::GiftWrap => {
            let keys_to_try = prioritized_signers_for_event(event, &client.signers);

            for (node_id, nostr_keys) in keys_to_try {
                // Try to unwrap the message with one of our signers
                if unwrap_direct_message(event, &nostr_keys).await.is_some() {
                    let signer = client.get_signer(&node_id)?;
                    return Ok((node_id, signer));
                }
            }
            Err(Error::Message(
                "No local identity could decrypt this message".to_string(),
            ))
        }
        Kind::TextNote | Kind::RelayList | Kind::Metadata => any_local_signer(client),
        kind if kind == BLOSSOM_SERVER_LIST_KIND || kind == FILE_METADATA_KIND => {
            any_local_signer(client)
        }
        _ => Err(Error::Message(format!(
            "Unsupported event kind: {:?}",
            event.kind
        ))),
    }
}

fn any_local_signer(client: &NostrClient) -> Result<(NodeId, Arc<Keys>)> {
    let node_id = client
        .signers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| Error::Message("No local identities available".to_string()))?;
    let signer = client.get_signer(&node_id)?;
    Ok((node_id, signer))
}

fn prioritized_signers_for_event(
    event: &Event,
    signers: &Arc<Mutex<HashMap<NodeId, Arc<Keys>>>>,
) -> Vec<(NodeId, Arc<Keys>)> {
    // Clone the keys to avoid holding the lock during async operations
    let mut keys_to_try: Vec<(NodeId, Arc<Keys>)> = signers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(id, keys)| (id.clone(), keys.clone()))
        .collect();

    if event.kind == Kind::GiftWrap
        && let Some(recipient_pubkey) = event.tags.public_keys().next()
        && let Some(index) = keys_to_try
            .iter()
            .position(|(_, nostr_keys)| nostr_keys.public_key() == recipient_pubkey)
        && index != 0
    {
        let matching_signer = keys_to_try.swap_remove(index);
        keys_to_try.insert(0, matching_signer);
    }

    keys_to_try
}

/// Detects event types and routes them to the correct handler.
pub async fn process_event(
    event: Box<Event>,
    signer: Arc<Keys>,
    client_id: NodeId,
    chain_key_store: Arc<dyn ChainKeyServiceApi>,
    event_handlers: &[Arc<dyn NotificationHandlerApi>],
    file_metadata_processor: Arc<dyn FileMetadataProcessorApi>,
) -> Result<(bool, Timestamp)> {
    let (success, time) = match event.kind {
        Kind::GiftWrap => {
            trace!("Received encrypted direct message: {event:?}");
            match handle_direct_message(event.clone(), &signer, &client_id, event_handlers).await {
                Err(e) => {
                    error!("Failed to handle direct message: {e}");
                    (false, Timestamp::zero())
                }
                Ok(_) => (true, event.created_at.into()),
            }
        }
        Kind::TextNote => {
            match handle_public_event(event.clone(), &client_id, &chain_key_store, event_handlers)
                .await
            {
                Err(e) => {
                    debug!("Skipping public chain event with missing chain keys: {e}");
                    (false, Timestamp::zero())
                }
                Ok(v) => {
                    if v {
                        (v, event.created_at.into())
                    } else {
                        (false, Timestamp::zero())
                    }
                }
            }
        }
        Kind::RelayList => {
            debug!("Received relay list from: {}", event.pubkey);
            (true, Timestamp::zero())
        }
        kind if kind == BLOSSOM_SERVER_LIST_KIND => {
            debug!("Received blossom server list from: {}", event.pubkey);
            (true, Timestamp::zero())
        }
        kind if kind == FILE_METADATA_KIND => {
            let result = file_metadata_processor
                .process_file_metadata(event.clone(), &client_id)
                .await;
            if let Err(e) = result {
                debug!("File metadata processor failed: {}", e);
                return Ok((false, Timestamp::zero()));
            }
            (true, event.created_at.into())
        }
        Kind::Metadata => {
            debug!("Received metadata from: {}", event.pubkey);
            (true, Timestamp::zero())
        }
        _ => (true, Timestamp::zero()),
    };

    Ok((success, time))
}

pub async fn should_process(
    event: Box<Event>,
    local_node_ids: &[NodeId],
    contact_service: &Arc<dyn ContactServiceApi>,
    offset_store: &Arc<dyn NostrEventOffsetStoreApi>,
    since: Option<Timestamp>,
) -> bool {
    valid_time(event.kind, event.created_at, since)
        && valid_sender(&event.pubkey, local_node_ids, contact_service).await
        && !offset_store
            .is_processed(&event.id.to_hex())
            .await
            .unwrap_or(false)
}

fn valid_time(kind: Kind, created: nostr::types::Timestamp, since: Option<Timestamp>) -> bool {
    match since {
        Some(time) if !matches!(kind, Kind::GiftWrap) => created.as_secs() >= time.inner(),
        _ => true,
    }
}

pub async fn handle_direct_message(
    event: Box<Event>,
    signer: &Keys,
    client_id: &NodeId,
    event_handlers: &[Arc<dyn NotificationHandlerApi>],
) -> Result<()> {
    if let Some((envelope, sender, _, _)) = unwrap_direct_message(&event, signer).await {
        let sender_npub = sender.to_bech32();
        let sender_pub_key = sender.to_hex();
        debug!(
            "Processing event: {} {} from {sender_npub:?} (hex: {sender_pub_key}) on client {client_id}",
            envelope.event_type, envelope.version
        );
        handle_event(envelope, client_id, event_handlers, Some(sender), event).await?;
    }
    Ok(())
}

async fn handle_public_event(
    event: Box<Event>,
    node_id: &NodeId,
    chain_key_store: &Arc<dyn ChainKeyServiceApi>,
    handlers: &[Arc<dyn NotificationHandlerApi>],
) -> Result<bool> {
    if let Some(encoded_or_encrypted) = unwrap_public_chain_event(event.as_ref())? {
        debug!(
            "Received public chain event: {} {}",
            encoded_or_encrypted.chain_type, encoded_or_encrypted.id
        );
        // make sure we have the correct keys for this event
        if let Ok(Some(keys)) = chain_key_store
            .get_chain_keys(&encoded_or_encrypted.id, encoded_or_encrypted.chain_type)
            .await
        {
            let decoded = decrypt_or_decode_public_chain_event(
                &encoded_or_encrypted.payload,
                &Some(keys.to_owned()),
            )?;
            debug!("Handling public chain event: {:?}", decoded.event_type);
            handle_event(
                decoded.clone(),
                node_id,
                handlers,
                Some(event.pubkey),
                event.clone(),
            )
            .await?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn valid_sender(
    npub: &PublicKey,
    local_node_ids: &[NodeId],
    contact_service: &Arc<dyn ContactServiceApi>,
) -> bool {
    if local_node_ids.iter().any(|node_id| node_id.npub() == *npub) {
        return true;
    }
    if let Ok(res) = contact_service.is_known_npub(npub).await {
        res
    } else {
        error!("Could not check if sender is a known contact");
        false
    }
}

async fn get_offset(db: &Arc<dyn NostrEventOffsetStoreApi>, node_id: &NodeId) -> Timestamp {
    db.current_offset(node_id)
        .await
        .map_err(|e| error!("Could not get event offset: {e}"))
        .ok()
        .unwrap_or(Timestamp::zero())
}

pub async fn add_offset(
    db: &Arc<dyn NostrEventOffsetStoreApi>,
    event_id: EventId,
    time: Timestamp,
    success: bool,
    node_id: &NodeId,
) {
    db.add_event(NostrEventOffset {
        event_id: event_id.to_hex(),
        time,
        success,
        node_id: node_id.to_owned(),
    })
    .await
    .map_err(|e| error!("Could not store event offset: {e}"))
    .ok();
}

/// Handle extracted event with given handlers.
async fn handle_event(
    event: EventEnvelope,
    node_id: &NodeId,
    handlers: &[Arc<dyn NotificationHandlerApi>],
    sender: Option<nostr::key::PublicKey>,
    original_event: Box<nostr::event::Event>,
) -> Result<()> {
    let event_type = &event.event_type;
    let mut times = 0;
    for handler in handlers.iter() {
        if handler.handles_event(event_type) {
            match handler
                .handle_event(
                    event.to_owned(),
                    node_id,
                    sender,
                    Some(original_event.clone()),
                )
                .await
            {
                Ok(_) => times += 1,
                Err(e) => error!("Nostr event handler failed: {e}"),
            }
        }
    }
    if times < 1 {
        warn!("No handler subscribed for event: {event:?}");
    } else {
        trace!("{event_type:?} event handled successfully {times} times");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use bcr_common::core::NodeId;
    use bcr_ebill_api::service::transport_service::transport_client::TransportClientApi;
    use bcr_ebill_core::protocol::Email;
    use bcr_ebill_core::protocol::Timestamp;
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_ebill_core::protocol::crypto::BcrKeys;
    use bcr_ebill_core::protocol::event::BillEventType;
    use bcr_ebill_core::protocol::event::{Event, EventType};
    use bcr_ebill_persistence::NostrEventOffset;
    use mockall::predicate;
    use nostr::{
        event::{EventBuilder, FinalizeEvent, FinalizeUnsignedEvent},
        nips::nip59::GiftWrapBuilder,
    };
    use tokio::time;

    use crate::test_utils::{
        MockContactService, MockFileMetadataProcessor, MockNostrEventOffsetStore, TestEventPayload,
        create_test_event, get_identity_public_data,
    };
    use crate::{handler::MockNotificationHandlerApi, test_utils::MockChainKeyService};

    use super::super::test_utils::get_mock_relay;
    use super::{NostrClient, NostrConfig, NostrConsumer, prioritized_signers_for_event};

    #[tokio::test]
    async fn test_connect() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let keys = BcrKeys::new();
        let config = NostrConfig::new(
            keys.clone(),
            vec![url.to_owned()],
            vec![],
            true,
            NodeId::new(keys.pub_key(), bitcoin::Network::Testnet),
        );
        let client = NostrClient::default(&config)
            .await
            .expect("failed to create nostr client");

        client.connect().await.expect("failed to connect");
        assert!(client.is_connected(), "client should be connected");
    }

    #[tokio::test]
    async fn test_has_connected_relays_reflects_runtime_state() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let keys = BcrKeys::new();
        let config = NostrConfig::new(
            keys.clone(),
            vec![url],
            vec![],
            true,
            NodeId::new(keys.pub_key(), bitcoin::Network::Testnet),
        );
        let client = NostrClient::default(&config)
            .await
            .expect("failed to create nostr client");

        assert!(
            !client.has_connected_relays().await,
            "no relay should be connected before connect"
        );

        client.connect().await.expect("failed to connect");

        let timeout = Duration::from_secs(3);
        let start = Instant::now();
        while !client.has_connected_relays().await && start.elapsed() < timeout {
            time::sleep(Duration::from_millis(50)).await;
        }

        assert!(
            client.has_connected_relays().await,
            "at least one relay should be connected after connect"
        );
    }

    /// When testing with the mock relay we need to be careful. It is always
    /// listening on the same port and will not start multiple times. If we
    /// share the instance tests will fail with events from other tests.
    #[tokio::test]
    async fn test_send_and_receive_event() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        let keys1 = BcrKeys::new();
        let keys2 = BcrKeys::new();

        // given two clients
        let config1 = NostrConfig::new(
            keys1.clone(),
            vec![url.to_owned()],
            vec![],
            true,
            NodeId::new(keys1.pub_key(), bitcoin::Network::Testnet),
        );
        let client1 = NostrClient::default(&config1)
            .await
            .expect("failed to create nostr client 1");

        client1.connect().await.expect("failed to connect");

        let config2 = NostrConfig::new(
            keys2.clone(),
            vec![url.to_owned()],
            vec![],
            true,
            NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet),
        );
        let client2 = NostrClient::default(&config2)
            .await
            .expect("failed to create nostr client 2");

        client2.connect().await.expect("failed to connect");

        // and a contact we want to send an event to
        let contact = get_identity_public_data(
            &NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet),
            &Email::new("payee@example.com").unwrap(),
            vec![&url],
        );
        let event = create_test_event(&BillEventType::BillSigned);

        // expect the receiver to check if the sender contact is known
        let mut contact_service = MockContactService::new();
        contact_service
            .expect_is_known_npub()
            .with(predicate::eq(keys1.get_nostr_keys().public_key()))
            .returning(|_| Ok(true));

        // expect a handler that is subscribed to the event type w sent
        let mut handler = MockNotificationHandlerApi::new();
        handler
            .expect_handles_event()
            .with(predicate::eq(&EventType::Bill))
            .returning(|_| true);

        // expect a handler receiving the event we sent
        let expected_event: Event<TestEventPayload> = event.clone();
        handler
            .expect_handle_event()
            .withf(move |e, i, _, _| {
                let expected = expected_event.clone();
                let received: Event<TestEventPayload> =
                    e.clone().try_into().expect("could not convert event");
                let valid_type = received.event_type == expected.event_type;
                let valid_payload = received.data.foo == expected.data.foo;
                let valid_identity = *i == NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);
                valid_type && valid_payload && valid_identity
            })
            .returning(|_, _, _, _| Ok(()));

        let mut offset_store = MockNostrEventOffsetStore::new();

        // expect the offset store to return the current offset once on start
        offset_store
            .expect_current_offset()
            .returning(|_| Ok(Timestamp::new(1000).unwrap()))
            .once();

        // should also check if the event has been processed already
        offset_store
            .expect_is_processed()
            .withf(|e: &str| !e.is_empty())
            .returning(|_| Ok(false))
            .once();

        // when done processing the event, add it to the offset store
        offset_store
            .expect_add_event()
            .withf(|e: &NostrEventOffset| e.success)
            .returning(|_| Ok(()))
            .once();

        let chain_key_store = MockChainKeyService::new();

        // we start the consumer
        let consumer = NostrConsumer::new(
            Arc::new(client2),
            Arc::new(contact_service),
            vec![Arc::new(handler)],
            Arc::new(offset_store),
            Arc::new(chain_key_store),
            Arc::new(MockFileMetadataProcessor::new()),
        );

        // run in a local set
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle = tokio::task::spawn_local(async move {
                    consumer
                        .start()
                        .await
                        .expect("failed to start nostr consumer");
                });
                // and send an event
                let node_id1 = NodeId::new(keys1.pub_key(), bitcoin::Network::Testnet);
                client1
                    .send_private_event(
                        &node_id1,
                        &BillParticipant::Ident(contact),
                        event.try_into().expect("could not convert event"),
                    )
                    .await
                    .expect("failed to send event");

                // give it a little bit of time to process the event
                time::sleep(Duration::from_millis(100)).await;
                handle.abort();
            })
            .await;
    }

    #[tokio::test]
    async fn test_multi_identity_client() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        let keys1 = BcrKeys::new();
        let keys2 = BcrKeys::new();
        let node_id1 = NodeId::new(keys1.pub_key(), bitcoin::Network::Testnet);
        let node_id2 = NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);

        let identities = vec![
            (node_id1.clone(), keys1.clone()),
            (node_id2.clone(), keys2.clone()),
        ];

        let client = NostrClient::new(
            identities,
            vec![url],
            vec![],
            Duration::from_secs(20),
            None,
            1,
            None,
        )
        .await
        .expect("failed to create multi-identity client");

        // Should be able to get signer for each identity
        assert!(client.get_signer(&node_id1).is_ok());
        assert!(client.get_signer(&node_id2).is_ok());

        // Should fail for unknown identity
        let unknown = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        assert!(client.get_signer(&unknown).is_err());
    }

    #[tokio::test]
    async fn test_send_private_event_with_sender_node_id() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        let keys1 = BcrKeys::new();
        let keys2 = BcrKeys::new();
        let node_id1 = NodeId::new(keys1.pub_key(), bitcoin::Network::Testnet);
        let node_id2 = NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);

        let identities = vec![
            (node_id1.clone(), keys1.clone()),
            (node_id2.clone(), keys2.clone()),
        ];

        let client = NostrClient::new(
            identities,
            vec![url.clone()],
            vec![],
            Duration::from_secs(20),
            None,
            1,
            None,
        )
        .await
        .expect("failed to create client");

        client.connect().await.expect("failed to connect");

        let recipient = get_identity_public_data(
            &node_id2,
            &Email::new("recipient@example.com").unwrap(),
            vec![&url],
        );
        let event = create_test_event(&BillEventType::BillSigned);

        // NIP-17 now supports multi-identity clients via manual gift wrap construction
        let result = client
            .send_private_event(
                &node_id1,
                &BillParticipant::Ident(recipient),
                event.try_into().unwrap(),
            )
            .await;

        assert!(
            result.is_ok(),
            "NIP-17 should work for multi-identity clients: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_prioritized_signers_for_gift_wrap_uses_tagged_recipient_first() {
        let sender_keys = BcrKeys::new();
        let recipient_keys = BcrKeys::new();
        let other_keys = BcrKeys::new();

        let recipient_node_id = NodeId::new(recipient_keys.pub_key(), bitcoin::Network::Testnet);
        let other_node_id = NodeId::new(other_keys.pub_key(), bitcoin::Network::Testnet);

        let signers = Arc::new(Mutex::new(HashMap::from([
            (other_node_id.clone(), Arc::new(other_keys.get_nostr_keys())),
            (
                recipient_node_id.clone(),
                Arc::new(recipient_keys.get_nostr_keys()),
            ),
        ])));

        let rumor = EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize_unsigned(sender_keys.get_nostr_keys().public_key());
        let event = GiftWrapBuilder::new(recipient_keys.get_nostr_keys().public_key(), rumor)
            .finalize(&sender_keys.get_nostr_keys())
            .expect("failed to build gift wrap event");

        let prioritized = prioritized_signers_for_event(&event, &signers);

        assert_eq!(prioritized.len(), 2);
        assert_eq!(prioritized[0].0, recipient_node_id);
        assert_eq!(prioritized[1].0, other_node_id);
    }

    #[tokio::test]
    async fn test_prioritized_signers_for_gift_wrap_keeps_fallback_when_tagged_recipient_missing() {
        let sender_keys = BcrKeys::new();
        let remote_recipient_keys = BcrKeys::new();
        let first_local_keys = BcrKeys::new();
        let second_local_keys = BcrKeys::new();

        let first_local_node_id =
            NodeId::new(first_local_keys.pub_key(), bitcoin::Network::Testnet);
        let second_local_node_id =
            NodeId::new(second_local_keys.pub_key(), bitcoin::Network::Testnet);

        let signers = Arc::new(Mutex::new(HashMap::from([
            (
                first_local_node_id.clone(),
                Arc::new(first_local_keys.get_nostr_keys()),
            ),
            (
                second_local_node_id.clone(),
                Arc::new(second_local_keys.get_nostr_keys()),
            ),
        ])));

        let rumor = EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize_unsigned(sender_keys.get_nostr_keys().public_key());
        let event =
            GiftWrapBuilder::new(remote_recipient_keys.get_nostr_keys().public_key(), rumor)
                .finalize(&sender_keys.get_nostr_keys())
                .expect("failed to build gift wrap event");

        let prioritized = prioritized_signers_for_event(&event, &signers);

        assert_eq!(prioritized.len(), 2);
        assert!(
            prioritized
                .iter()
                .any(|(node_id, _)| *node_id == first_local_node_id)
        );
        assert!(
            prioritized
                .iter()
                .any(|(node_id, _)| *node_id == second_local_node_id)
        );
        assert!(prioritized.iter().all(|(_, keys)| {
            keys.public_key() != remote_recipient_keys.get_nostr_keys().public_key()
        }));
    }

    #[tokio::test]
    async fn test_publish_and_fetch_blossom_server_list() {
        let relay = get_mock_relay().await;
        let relay_url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        let sender_keys = BcrKeys::new();
        let sender_node_id = NodeId::new(sender_keys.pub_key(), bitcoin::Network::Testnet);
        let sender_config = NostrConfig::new(
            sender_keys.clone(),
            vec![relay_url.clone()],
            vec![],
            true,
            sender_node_id.clone(),
        );
        let sender = NostrClient::default(&sender_config).await.unwrap();
        sender.connect().await.unwrap();

        let receiver_keys = BcrKeys::new();
        let receiver_node_id = NodeId::new(receiver_keys.pub_key(), bitcoin::Network::Testnet);
        let receiver_config = NostrConfig::new(
            receiver_keys,
            vec![relay_url.clone()],
            vec![],
            true,
            receiver_node_id,
        );
        let receiver = NostrClient::default(&receiver_config).await.unwrap();
        receiver.connect().await.unwrap();

        let expected = vec![
            url::Url::parse("https://blossom-one.example.com").unwrap(),
            url::Url::parse("https://blossom-two.example.com").unwrap(),
        ];

        sender
            .publish_blossom_server_list(&sender_node_id, expected.clone())
            .await
            .unwrap();
        time::sleep(Duration::from_millis(100)).await;

        let fetched = receiver
            .fetch_blossom_server_list(sender_node_id.npub(), vec![relay_url])
            .await
            .unwrap();

        assert_eq!(fetched, expected);
    }

    #[tokio::test]
    async fn test_nostr_consumer_receives_blossom_server_list_event() {
        let relay = get_mock_relay().await;
        let relay_url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        let sender_keys = BcrKeys::new();
        let sender_node_id = NodeId::new(sender_keys.pub_key(), bitcoin::Network::Testnet);
        let sender_config = NostrConfig::new(
            sender_keys.clone(),
            vec![relay_url.clone()],
            vec![],
            true,
            sender_node_id.clone(),
        );
        let sender = NostrClient::default(&sender_config).await.unwrap();
        sender.connect().await.unwrap();

        let receiver_keys = BcrKeys::new();
        let receiver_node_id = NodeId::new(receiver_keys.pub_key(), bitcoin::Network::Testnet);
        let receiver_config = NostrConfig::new(
            receiver_keys,
            vec![relay_url.clone()],
            vec![],
            true,
            receiver_node_id,
        );
        let receiver = Arc::new(NostrClient::default(&receiver_config).await.unwrap());

        let mut contact_service = MockContactService::new();
        let sender_npub = sender_node_id.npub();
        contact_service
            .expect_get_nostr_npubs()
            .returning(move || Ok(vec![sender_npub]))
            .once();
        contact_service
            .expect_is_known_npub()
            .with(predicate::eq(sender_npub))
            .returning(|_| Ok(true))
            .once();

        let mut offset_store = MockNostrEventOffsetStore::new();
        offset_store
            .expect_current_offset()
            .returning(|_| Ok(Timestamp::zero()));
        offset_store
            .expect_is_processed()
            .returning(|_| Ok(false))
            .once();
        offset_store
            .expect_add_event()
            .withf(|event: &NostrEventOffset| event.success)
            .returning(|_| Ok(()))
            .once();

        let consumer = NostrConsumer::new(
            receiver,
            Arc::new(contact_service),
            vec![],
            Arc::new(offset_store),
            Arc::new(MockChainKeyService::new()),
            Arc::new(MockFileMetadataProcessor::new()),
        );

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async move {
                let handle = tokio::task::spawn_local(async move {
                    consumer
                        .start()
                        .await
                        .expect("failed to start nostr consumer");
                });

                sender
                    .publish_blossom_server_list(
                        &sender_node_id,
                        vec![url::Url::parse("https://blossom.example.com").unwrap()],
                    )
                    .await
                    .unwrap();

                time::sleep(Duration::from_millis(150)).await;
                handle.abort();
            })
            .await;
    }

    #[tokio::test]
    async fn test_nostr_consumer_single_client_multi_identity() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();

        // Create two identities
        let keys1 = BcrKeys::new();
        let keys2 = BcrKeys::new();
        let node_id1 = NodeId::new(keys1.pub_key(), bitcoin::Network::Testnet);
        let node_id2 = NodeId::new(keys2.pub_key(), bitcoin::Network::Testnet);

        // Create single client with multiple identities
        let identities = vec![
            (node_id1.clone(), keys1.clone()),
            (node_id2.clone(), keys2.clone()),
        ];

        let client = Arc::new(
            NostrClient::new(
                identities,
                vec![url.clone()],
                vec![],
                Duration::from_secs(20),
                None,
                1,
                None,
            )
            .await
            .expect("failed to create multi-identity client"),
        );

        // Create mock services for NostrConsumer with expectations
        let mut contact_service = MockContactService::new();
        contact_service
            .expect_get_nostr_npubs()
            .returning(|| Ok(vec![]));

        let mut offset_store = MockNostrEventOffsetStore::new();
        // Set expectations for both node IDs
        offset_store
            .expect_current_offset()
            .returning(|_| Ok(Timestamp::zero()));

        let chain_key_service = Arc::new(MockChainKeyService::new());

        // Create NostrConsumer with single client (not Vec of clients)
        let consumer = NostrConsumer::new(
            client,
            Arc::new(contact_service),
            vec![],
            Arc::new(offset_store),
            chain_key_service,
            Arc::new(MockFileMetadataProcessor::new()),
        );

        // Verify consumer can start and subscribe to events for all identities
        let mut tasks = consumer.start().await.expect("failed to start consumer");
        assert_eq!(tasks.len(), 1, "Should have single task for single client");

        // Clean up tasks
        tasks.abort_all();
    }
}

/// Internal relay calculation function (pure function for testing)
fn calculate_relay_set_internal(
    user_relays: &[url::Url],
    contacts: &[NostrContact],
    max_relays: Option<usize>,
) -> HashSet<url::Url> {
    let mut relay_set = HashSet::new();

    // Pass 1: Add all user relays (exempt from limit)
    for relay in user_relays {
        relay_set.insert(relay.clone());
    }

    // Filter and sort contacts by trust level
    let mut eligible_contacts: Vec<&NostrContact> = contacts
        .iter()
        .filter(|c| matches!(c.trust_level, TrustLevel::Trusted | TrustLevel::Participant))
        .collect();

    // Sort: Trusted (0) before Participant (1)
    eligible_contacts.sort_by_key(|c| match c.trust_level {
        TrustLevel::Trusted => 0,
        TrustLevel::Participant => 1,
        _ => 2, // unreachable due to filter
    });

    let contact_relay_limit = NOSTR_MAX_RELAYS.min(max_relays.unwrap_or(NOSTR_MAX_RELAYS));
    let user_relay_count = relay_set.len();

    // Pass 2: Add first relay from each contact (priority order)
    for contact in &eligible_contacts {
        let contact_relays_added = relay_set.len() - user_relay_count;
        if contact_relays_added >= contact_relay_limit {
            break;
        }
        if let Some(first_relay) = contact.relays.first() {
            relay_set.insert(first_relay.clone());
        }
    }

    // Pass 3: Fill remaining slots with additional contact relays
    for contact in &eligible_contacts {
        for relay in contact.relays.iter().skip(1) {
            let contact_relays_added = relay_set.len() - user_relay_count;
            if contact_relays_added >= contact_relay_limit {
                return relay_set;
            }
            relay_set.insert(relay.clone());
        }
    }

    relay_set
}

#[cfg(test)]
mod relay_calculation_tests {
    use super::*;
    use bcr_ebill_core::application::nostr_contact::{HandshakeStatus, NostrContact, TrustLevel};

    fn create_test_contact(trust_level: TrustLevel, relays: Vec<&str>) -> NostrContact {
        use bcr_ebill_core::protocol::crypto::BcrKeys;
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        NostrContact {
            npub: node_id.npub(),
            node_id,
            name: None,
            relays: relays.iter().map(|r| url::Url::parse(r).unwrap()).collect(),
            blossom_servers: vec![],
            trust_level,
            handshake_status: HandshakeStatus::None,
            contact_private_key: None,
            mint_url: None,
        }
    }

    #[test]
    fn test_user_relays_always_included() {
        let user_relays = vec![
            url::Url::parse("wss://relay1.com").unwrap(),
            url::Url::parse("wss://relay2.com").unwrap(),
        ];
        let contacts = vec![];
        let max_relays = Some(1); // Very low limit

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // User relays should all be present despite low limit
        assert_eq!(result.len(), 2);
        assert!(result.contains(&url::Url::parse("wss://relay1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://relay2.com").unwrap()));
    }

    #[test]
    fn test_trusted_contacts_prioritized() {
        let user_relays = vec![];
        let contacts = vec![
            create_test_contact(TrustLevel::Participant, vec!["wss://participant.com"]),
            create_test_contact(TrustLevel::Trusted, vec!["wss://trusted.com"]),
        ];
        let max_relays = Some(1);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // Should only include trusted contact's relay (higher priority)
        assert_eq!(result.len(), 1);
        assert!(result.contains(&url::Url::parse("wss://trusted.com").unwrap()));
    }

    #[test]
    fn test_contact_relays_added_when_user_relays_exceed_limit() {
        let user_relays = vec![
            url::Url::parse("wss://user1.com").unwrap(),
            url::Url::parse("wss://user2.com").unwrap(),
            url::Url::parse("wss://user3.com").unwrap(),
        ];
        let contacts = vec![
            create_test_contact(TrustLevel::Trusted, vec!["wss://contact1.com"]),
            create_test_contact(TrustLevel::Trusted, vec!["wss://contact2.com"]),
        ];
        let max_relays = Some(2); // Lower than user relay count

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // Should have all 3 user relays + 2 contact relays (user relays exempt from limit)
        assert_eq!(result.len(), 5);
        assert!(result.contains(&url::Url::parse("wss://user1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://user2.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://user3.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://contact1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://contact2.com").unwrap()));
    }

    #[test]
    fn test_one_relay_per_contact_guaranteed() {
        let user_relays = vec![];
        let contacts = vec![
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://contact1-relay1.com", "wss://contact1-relay2.com"],
            ),
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://contact2-relay1.com", "wss://contact2-relay2.com"],
            ),
            create_test_contact(TrustLevel::Trusted, vec!["wss://contact3-relay1.com"]),
        ];
        let max_relays = Some(3);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // Should have exactly 3 relays (first relay from each contact)
        assert_eq!(result.len(), 3);
        assert!(result.contains(&url::Url::parse("wss://contact1-relay1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://contact2-relay1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://contact3-relay1.com").unwrap()));
    }

    #[test]
    fn test_deduplication_across_contacts() {
        let user_relays = vec![];
        let contacts = vec![
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://shared.com", "wss://unique1.com"],
            ),
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://shared.com", "wss://unique2.com"],
            ),
        ];
        let max_relays = Some(10);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // Should only include shared.com once
        assert_eq!(result.len(), 3);
        assert!(result.contains(&url::Url::parse("wss://shared.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://unique1.com").unwrap()));
        assert!(result.contains(&url::Url::parse("wss://unique2.com").unwrap()));
    }

    #[test]
    fn test_banned_contacts_excluded() {
        let user_relays = vec![];
        let contacts = vec![
            create_test_contact(TrustLevel::Banned, vec!["wss://banned.com"]),
            create_test_contact(TrustLevel::Trusted, vec!["wss://trusted.com"]),
        ];
        let max_relays = Some(10);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        assert_eq!(result.len(), 1);
        assert!(result.contains(&url::Url::parse("wss://trusted.com").unwrap()));
        assert!(!result.contains(&url::Url::parse("wss://banned.com").unwrap()));
    }

    #[test]
    fn test_none_trust_level_excluded() {
        let user_relays = vec![];
        let contacts = vec![
            create_test_contact(TrustLevel::None, vec!["wss://unknown.com"]),
            create_test_contact(TrustLevel::Participant, vec!["wss://participant.com"]),
        ];
        let max_relays = Some(10);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        assert_eq!(result.len(), 1);
        assert!(result.contains(&url::Url::parse("wss://participant.com").unwrap()));
        assert!(!result.contains(&url::Url::parse("wss://unknown.com").unwrap()));
    }

    #[test]
    fn test_no_limit_when_max_relays_none() {
        let user_relays = vec![url::Url::parse("wss://user.com").unwrap()];
        let contacts = vec![
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://relay1.com", "wss://relay2.com"],
            ),
            create_test_contact(
                TrustLevel::Trusted,
                vec!["wss://relay3.com", "wss://relay4.com"],
            ),
        ];
        let max_relays = None;

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // All relays should be included
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_empty_contacts() {
        let user_relays = vec![url::Url::parse("wss://user.com").unwrap()];
        let contacts = vec![];
        let max_relays = Some(50);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        assert_eq!(result.len(), 1);
        assert!(result.contains(&url::Url::parse("wss://user.com").unwrap()));
    }

    #[test]
    fn test_contact_with_no_relays() {
        let user_relays = vec![];
        let mut contact = create_test_contact(TrustLevel::Trusted, vec![]);
        contact.relays = vec![]; // Explicitly no relays
        let contacts = vec![contact];
        let max_relays = Some(10);

        let result = calculate_relay_set_internal(&user_relays, &contacts, max_relays);

        // Should handle gracefully
        assert_eq!(result.len(), 0);
    }
}

#[cfg(test)]
mod optimistic_broadcast_tests {
    use super::super::test_utils::{MockNostrContactStore, get_mock_relay};
    use super::{NostrClient, NostrConfig};
    use bcr_common::core::NodeId;
    use bcr_ebill_api::service::transport_service::transport_client::TransportClientApi;
    use bcr_ebill_core::protocol::crypto::BcrKeys;
    use nostr::event::{EventBuilder, FinalizeEvent};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn test_relay_ack_threshold_returns_configured_value() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let keys = BcrKeys::new();
        let config = NostrConfig::new(
            keys.clone(),
            vec![url],
            vec![],
            true,
            NodeId::new(keys.pub_key(), bitcoin::Network::Testnet),
        )
        .with_relay_ack_threshold(3);
        let client = NostrClient::default(&config)
            .await
            .expect("failed to create nostr client");

        assert_eq!(client.relay_ack_threshold(), 3);
    }

    #[tokio::test]
    async fn test_broadcast_event_optimistic_returns_after_ack() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let keys = BcrKeys::new();
        let config = NostrConfig::new(
            keys.clone(),
            vec![url],
            vec![],
            true,
            NodeId::new(keys.pub_key(), bitcoin::Network::Testnet),
        );
        let client = NostrClient::default(&config)
            .await
            .expect("failed to create nostr client");

        client.connect().await.expect("failed to connect");

        let event = EventBuilder::new(nostr::event::Kind::TextNote, "test optimistic broadcast")
            .finalize(&keys.get_nostr_keys())
            .unwrap();

        let result = client.broadcast_event_optimistic(&event, 1).await;
        assert!(
            result.is_ok(),
            "optimistic broadcast should return after first ack: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn test_broadcast_event_optimistic_clamps_threshold_to_relay_count() {
        let relay = get_mock_relay().await;
        let url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let keys = BcrKeys::new();
        let config = NostrConfig::new(
            keys.clone(),
            vec![url],
            vec![],
            true,
            NodeId::new(keys.pub_key(), bitcoin::Network::Testnet),
        );
        let client = NostrClient::default(&config)
            .await
            .expect("failed to create nostr client");

        client.connect().await.expect("failed to connect");

        let event = EventBuilder::new(nostr::event::Kind::TextNote, "test threshold clamping")
            .finalize(&keys.get_nostr_keys())
            .unwrap();

        let result = client.broadcast_event_optimistic(&event, 5).await;
        assert!(
            result.is_ok(),
            "optimistic broadcast should clamp threshold to relay count: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    #[ignore] // slow test - enable on-demand
    async fn test_pre_threshold_relay_failure_is_queued_for_resync() {
        let relay = get_mock_relay().await;
        let real_url = url::Url::parse(&relay.url().await.to_string()).unwrap();
        let unreachable_url = url::Url::parse("ws://127.0.0.1:1").unwrap();
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);

        let mut mock_store = MockNostrContactStore::new();
        let unreachable_url_for_assert = unreachable_url.clone();
        mock_store
            .expect_add_failed_relay_sync()
            .withf(move |url, _event| url.as_str() == unreachable_url_for_assert.as_str())
            .times(1..)
            .returning(|_, _| Ok(()));

        let client = NostrClient::new(
            vec![(node_id, keys.clone())],
            vec![real_url, unreachable_url],
            vec![],
            Duration::from_secs(2),
            Some(10),
            2,
            Some(Arc::new(mock_store)),
        )
        .await
        .expect("failed to create nostr client");

        client.connect().await.expect("failed to connect");

        let event = EventBuilder::new(nostr::event::Kind::TextNote, "pre-threshold failure")
            .finalize(&keys.get_nostr_keys())
            .unwrap();

        let result = client.broadcast_event_optimistic(&event, 2).await;
        assert!(
            result.is_err(),
            "threshold should not be met because one relay is unreachable"
        );
    }
}
