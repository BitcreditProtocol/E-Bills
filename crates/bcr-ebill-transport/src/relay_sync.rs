use crate::nostr::NostrClient;
use bcr_ebill_api::{
    constants::{RELAY_SYNC_EVENT_DELAY_MS, RELAY_SYNC_GAP_THRESHOLD_SECONDS},
    service::transport_service::{Error, Result},
};
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_persistence::nostr::{NostrStoreApi, SyncStatus};
use futures::future::join_all;
use log::{debug, error, info, warn};
use nostr::{event::Kind, filter::Filter, key::PublicKey};
use std::sync::Arc;
use std::time::Duration;
use tokio_with_wasm::alias as tokio;

/// Helper to convert persistence errors to transport errors
fn to_transport_error(e: bcr_ebill_persistence::Error) -> Error {
    Error::Message(format!("Persistence error: {}", e))
}

/// Type of filter for querying events
enum FilterType {
    Pubkey, // For private messages TO us
    Author, // For public events FROM us
}

/// Syncs all pending relays by streaming events and publishing to targets
pub async fn sync_pending_relays(
    client: &NostrClient,
    user_relays: &[url::Url],
    nostr_store: &Arc<dyn NostrStoreApi>,
) -> Result<()> {
    // Get all relays that need syncing
    let pending_relays = nostr_store
        .get_pending_relays()
        .await
        .map_err(to_transport_error)?;

    if pending_relays.is_empty() {
        debug!("No pending relays to sync");
        return Ok(());
    }

    info!("Starting sync for {} relays", pending_relays.len());

    // Source relays = all user relays EXCEPT pending ones
    let source_relays: Vec<url::Url> = user_relays
        .iter()
        .filter(|r| !pending_relays.contains(r))
        .cloned()
        .collect();

    if source_relays.is_empty() {
        // All relays are new (e.g., new account) - nothing to sync from
        // Mark them all as Completed immediately
        info!(
            "No source relays available - marking all {} relays as Completed (new account)",
            pending_relays.len()
        );
        for relay in &pending_relays {
            nostr_store
                .update_relay_sync_status(relay, SyncStatus::Completed)
                .await
                .map_err(to_transport_error)?;
        }
        return Ok(());
    }

    // Mark all as InProgress
    for relay in &pending_relays {
        nostr_store
            .update_relay_sync_status(relay, SyncStatus::InProgress)
            .await
            .map_err(to_transport_error)?;
    }

    // Find earliest resume timestamp
    let earliest_timestamp =
        calculate_earliest_resume_timestamp(nostr_store, &pending_relays).await?;

    info!(
        "Syncing from {} source relays to {} target relays, starting from timestamp {}",
        source_relays.len(),
        pending_relays.len(),
        earliest_timestamp.inner()
    );

    // Get all identities' public keys
    let all_pubkeys: Vec<PublicKey> = client.get_all_node_ids().iter().map(|n| n.npub()).collect();

    // Sync private messages
    let private_synced = sync_event_type_to_multiple(
        client,
        &pending_relays,
        &source_relays,
        &all_pubkeys,
        vec![Kind::GiftWrap],
        FilterType::Pubkey,
        earliest_timestamp,
        nostr_store,
        Duration::from_millis(RELAY_SYNC_EVENT_DELAY_MS),
    )
    .await?;

    info!("Synced {} private messages", private_synced);

    // Sync public chain events
    let public_synced = sync_event_type_to_multiple(
        client,
        &pending_relays,
        &source_relays,
        &all_pubkeys,
        vec![Kind::TextNote],
        FilterType::Author,
        earliest_timestamp,
        nostr_store,
        Duration::from_millis(RELAY_SYNC_EVENT_DELAY_MS),
    )
    .await?;

    info!("Synced {} public chain events", public_synced);

    // Mark all as Completed
    for relay in &pending_relays {
        nostr_store
            .update_relay_sync_status(relay, SyncStatus::Completed)
            .await
            .map_err(to_transport_error)?;
    }

    info!("Relay sync completed successfully");
    Ok(())
}

/// Calculate the earliest resume timestamp across all pending relays
async fn calculate_earliest_resume_timestamp(
    nostr_store: &Arc<dyn NostrStoreApi>,
    pending_relays: &[url::Url],
) -> Result<Timestamp> {
    let mut earliest = Timestamp::now();

    for relay in pending_relays {
        if let Some(status) = nostr_store
            .get_relay_sync_status(relay)
            .await
            .map_err(to_transport_error)?
        {
            // Gap detection: Determine if relay was removed and re-added vs briefly offline.
            // If last_seen_in_config is more than RELAY_SYNC_GAP_THRESHOLD_SECONDS ago,
            // we consider it removed and re-added, so we start sync from current time
            // instead of resuming from last_synced_timestamp. This prevents syncing
            // large amounts of historical data when a relay is deliberately removed and
            // later re-added to the configuration.
            let gap_threshold = Timestamp::now().inner() - RELAY_SYNC_GAP_THRESHOLD_SECONDS;
            let has_gap = status.last_seen_in_config.inner() < gap_threshold;

            if let Some(last_synced) = status.last_synced_timestamp {
                if !has_gap && last_synced < earliest {
                    earliest = last_synced;
                }
            } else {
                // No previous sync, start from beginning
                earliest = Timestamp::zero();
                break;
            }
        } else {
            // No status record, start from beginning
            earliest = Timestamp::zero();
            break;
        }
    }

    Ok(earliest)
}

/// Sync a specific event type to multiple target relays
async fn sync_event_type_to_multiple(
    client: &NostrClient,
    target_relays: &[url::Url],
    source_relays: &[url::Url],
    pubkeys: &[PublicKey],
    kinds: Vec<Kind>,
    filter_type: FilterType,
    since: Timestamp,
    nostr_store: &Arc<dyn NostrStoreApi>,
    delay: Duration,
) -> Result<usize> {
    use futures::StreamExt;

    let filter = match filter_type {
        FilterType::Pubkey => Filter::new().pubkeys(pubkeys.to_vec()),
        FilterType::Author => Filter::new().authors(pubkeys.to_vec()),
    }
    .kinds(kinds)
    .since(since.into());

    // Stream events from source relays - more efficient for large result sets
    let event_stream = client
        .stream_events_from(filter, Some(source_relays.to_vec()), None)
        .await?;
    futures::pin_mut!(event_stream);

    let mut total_synced = 0;

    // Process events as they arrive from the stream
    while let Some(event) = event_stream.next().await {
        // Publish to all target relays in parallel using futures
        let mut futures_list = Vec::new();

        for target_relay in target_relays {
            // Check if this relay should skip this event
            if should_skip_event(nostr_store, target_relay, &event).await? {
                continue;
            }

            let client_clone = client.clone();
            let target = target_relay.clone();
            let evt = event.clone();
            let store = nostr_store.clone();

            let future = async move {
                match client_clone.send_event_to(vec![target.clone()], &evt).await {
                    Ok(_) => {
                        // Update progress
                        if let Err(e) = store
                            .update_relay_sync_progress(&target, evt.created_at.into())
                            .await
                        {
                            warn!("Failed to update sync progress for {}: {}", target, e);
                        }
                        Ok(())
                    }
                    Err(e) => {
                        warn!("Failed to sync event {} to {}: {}", evt.id, target, e);
                        // Add to retry queue - if this fails, the event is lost
                        if let Err(retry_err) =
                            store.add_failed_relay_sync(&target, evt.clone()).await
                        {
                            error!(
                                "CRITICAL: Failed to add event {} to retry queue for {}: {}. Event will be lost!",
                                evt.id, target, retry_err
                            );
                        }
                        Err(e)
                    }
                }
            };

            futures_list.push(future);
        }

        // Wait for all publishes to complete concurrently
        let results = join_all(futures_list).await;
        for result in results {
            if result.is_ok() {
                total_synced += 1;
            }
        }

        // Rate limiting - tokio_with_wasm::time::sleep works in both WASM and native
        if delay.as_millis() > 0 {
            tokio::time::sleep(delay).await;
        }
    }

    Ok(total_synced)
}

/// Check if an event should be skipped (already synced)
async fn should_skip_event(
    nostr_store: &Arc<dyn NostrStoreApi>,
    relay: &url::Url,
    event: &nostr::event::Event,
) -> Result<bool> {
    if let Some(status) = nostr_store
        .get_relay_sync_status(relay)
        .await
        .map_err(to_transport_error)?
        && let Some(last_ts) = status.last_synced_timestamp
    {
        let event_ts: Timestamp = event.created_at.into();
        return Ok(event_ts <= last_ts);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockNostrContactStore;
    use bcr_ebill_persistence::nostr::{NostrStoreApi, RelaySyncStatus};

    use bcr_common::core::NodeId;
    use mockall::predicate::eq;
    use nostr::event::FinalizeEvent;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_sync_pending_relays_no_pending() {
        let mut mock_store = MockNostrContactStore::new();

        // Expect get_pending_relays to return empty vec
        mock_store
            .expect_get_pending_relays()
            .times(1)
            .returning(|| Ok(vec![]));

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let user_relays = vec![url::Url::parse("wss://relay.example.com").unwrap()];

        // Mock client - won't be used since no pending relays
        let keys = bcr_ebill_core::protocol::crypto::BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let client = crate::nostr::NostrClient::new(
            vec![(node_id, keys)],
            user_relays.clone(),
            vec![],
            std::time::Duration::from_secs(10),
            None,
            1,
            Some(store.clone()),
        )
        .await
        .unwrap();

        // Should succeed without error when no pending relays
        let result = sync_pending_relays(&client, &user_relays, &store).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_sync_pending_relays_new_account_marks_completed() {
        let relay1 = url::Url::parse("wss://relay1.example.com").unwrap();
        let relay2 = url::Url::parse("wss://relay2.example.com").unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_pending_relays to return both relays
        let pending_relays = vec![relay1.clone(), relay2.clone()];
        mock_store
            .expect_get_pending_relays()
            .times(1)
            .returning(move || Ok(pending_relays.clone()));

        // Expect update_relay_sync_status calls
        mock_store
            .expect_update_relay_sync_status()
            .with(eq(relay1.clone()), eq(SyncStatus::Completed))
            .returning(|_, _| Ok(()))
            .once();
        mock_store
            .expect_update_relay_sync_status()
            .with(eq(relay2.clone()), eq(SyncStatus::Completed))
            .returning(|_, _| Ok(()))
            .once();

        // Expect get_relay_sync_status calls
        let relay1_clone = relay1.clone();
        let relay2_clone = relay2.clone();

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let user_relays = vec![relay1_clone.clone(), relay2_clone.clone()];
        let keys = bcr_ebill_core::protocol::crypto::BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let client = crate::nostr::NostrClient::new(
            vec![(node_id, keys)],
            user_relays.clone(),
            vec![],
            std::time::Duration::from_secs(10),
            None,
            1,
            Some(store.clone()),
        )
        .await
        .unwrap();

        // All relays are pending, no source relays - should mark as Completed
        let result = sync_pending_relays(&client, &user_relays, &store).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_calculate_earliest_resume_timestamp_no_previous_sync() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let relay_clone = relay.clone();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return status with no last_synced_timestamp
        mock_store
            .expect_get_relay_sync_status()
            .returning(move |_| {
                Ok(Some(RelaySyncStatus {
                    relay_url: relay_clone.clone(),
                    last_seen_in_config: Timestamp::now(),
                    sync_status: SyncStatus::Pending,
                    events_synced: 0,
                    last_synced_timestamp: None,
                    last_error: None,
                }))
            });

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let earliest = calculate_earliest_resume_timestamp(&store, &[relay])
            .await
            .unwrap();

        // Should return zero timestamp when no previous sync
        assert_eq!(earliest, Timestamp::zero());
    }

    #[tokio::test]
    async fn test_calculate_earliest_resume_timestamp_with_previous_sync() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let relay_clone = relay.clone();
        let timestamp = Timestamp::new(1000).unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return status with last_synced_timestamp
        mock_store
            .expect_get_relay_sync_status()
            .returning(move |_| {
                Ok(Some(RelaySyncStatus {
                    relay_url: relay_clone.clone(),
                    last_seen_in_config: Timestamp::now(),
                    sync_status: SyncStatus::Pending,
                    events_synced: 1,
                    last_synced_timestamp: Some(timestamp),
                    last_error: None,
                }))
            });

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let earliest = calculate_earliest_resume_timestamp(&store, &[relay])
            .await
            .unwrap();

        // Should return the last synced timestamp
        assert_eq!(earliest, timestamp);
    }

    #[tokio::test]
    async fn test_calculate_earliest_resume_timestamp_gap_detection() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let relay_clone = relay.clone();
        let old_timestamp = Timestamp::new(1000).unwrap();

        // Create status with old last_seen (> 24 hours ago)
        let very_old =
            Timestamp::new(Timestamp::now().inner() - RELAY_SYNC_GAP_THRESHOLD_SECONDS - 1000)
                .unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return status with old last_seen
        mock_store
            .expect_get_relay_sync_status()
            .returning(move |_| {
                Ok(Some(RelaySyncStatus {
                    relay_url: relay_clone.clone(),
                    last_seen_in_config: very_old,
                    sync_status: SyncStatus::Pending,
                    events_synced: 1,
                    last_synced_timestamp: Some(old_timestamp),
                    last_error: None,
                }))
            });

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let earliest = calculate_earliest_resume_timestamp(&store, &[relay])
            .await
            .unwrap();

        // Should NOT use the old timestamp due to gap, should start from current time
        assert!(earliest > old_timestamp);
    }

    #[tokio::test]
    async fn test_should_skip_event_no_status() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return None (no status)
        mock_store
            .expect_get_relay_sync_status()
            .times(1)
            .returning(|_| Ok(None));

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let keys = nostr::key::Keys::generate();
        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&keys)
            .unwrap();

        let should_skip = should_skip_event(&store, &relay, &event).await.unwrap();

        // No status means don't skip
        assert!(!should_skip);
    }

    #[tokio::test]
    async fn test_should_skip_event_already_synced() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let relay_clone = relay.clone();

        // Set last synced to a future timestamp
        let future_timestamp = Timestamp::new(Timestamp::now().inner() + 10000).unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return status with future timestamp
        mock_store
            .expect_get_relay_sync_status()
            .times(1)
            .returning(move |_| {
                Ok(Some(RelaySyncStatus {
                    relay_url: relay_clone.clone(),
                    last_seen_in_config: Timestamp::now(),
                    sync_status: SyncStatus::InProgress,
                    events_synced: 1,
                    last_synced_timestamp: Some(future_timestamp),
                    last_error: None,
                }))
            });

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let keys = nostr::key::Keys::generate();
        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&keys)
            .unwrap();

        let should_skip = should_skip_event(&store, &relay, &event).await.unwrap();

        // Event timestamp is before last_synced, should skip
        assert!(should_skip);
    }

    #[tokio::test]
    async fn test_should_skip_event_not_synced_yet() {
        let relay = url::Url::parse("wss://relay.example.com").unwrap();
        let relay_clone = relay.clone();

        // Set last synced to a past timestamp
        let past_timestamp = Timestamp::new(1000).unwrap();

        let mut mock_store = MockNostrContactStore::new();

        // Expect get_relay_sync_status to return status with past timestamp
        mock_store
            .expect_get_relay_sync_status()
            .times(1)
            .returning(move |_| {
                Ok(Some(RelaySyncStatus {
                    relay_url: relay_clone.clone(),
                    last_seen_in_config: Timestamp::now(),
                    sync_status: SyncStatus::InProgress,
                    events_synced: 1,
                    last_synced_timestamp: Some(past_timestamp),
                    last_error: None,
                }))
            });

        let store: Arc<dyn NostrStoreApi> = Arc::new(mock_store);
        let keys = nostr::key::Keys::generate();
        let event = nostr::event::EventBuilder::new(nostr::event::Kind::TextNote, "test")
            .finalize(&keys)
            .unwrap();

        let should_skip = should_skip_event(&store, &relay, &event).await.unwrap();

        // Event timestamp is after last_synced, should NOT skip
        assert!(!should_skip);
    }
}
