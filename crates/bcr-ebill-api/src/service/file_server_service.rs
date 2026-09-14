use crate::NostrConfig;
use crate::external::file_storage::{FileStorageClientApi, normalize_storage_base_url, to_url};
use crate::service::{Error, Result};
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bcr_ebill_persistence::FileReferenceStoreApi;
use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use log::{debug, warn};
use std::sync::Arc;

fn push_unique(urls: &mut Vec<url::Url>, url: url::Url) {
    if !urls.iter().any(|existing| existing == &url) {
        urls.push(url);
    }
}

/// Converts a relay URL into its Blossom HTTP endpoint form.
pub fn blossom_server_from_relay(relay_url: &url::Url) -> Result<url::Url> {
    normalize_storage_base_url(relay_url).map_err(Error::from)
}

/// Returns the configured Blossom servers, or derives a single fallback server from the first relay.
pub fn configured_blossom_servers(config: &NostrConfig) -> Vec<url::Url> {
    if !config.blossom_servers.is_empty() {
        return config.blossom_servers.clone();
    }

    config
        .relays
        .first()
        .and_then(|relay| blossom_server_from_relay(relay).ok())
        .into_iter()
        .collect()
}

/// Chooses explicit Blossom servers when available, otherwise derives them from relay fallbacks.
pub fn resolve_blossom_servers(
    blossom_servers: &[url::Url],
    fallback_relays: &[url::Url],
) -> Vec<url::Url> {
    if !blossom_servers.is_empty() {
        return blossom_servers.to_vec();
    }

    fallback_relays
        .first()
        .and_then(|relay| blossom_server_from_relay(relay).ok())
        .into_iter()
        .collect()
}

/// Merges multiple Blossom server lists while preserving order and removing duplicates.
pub fn merge_blossom_servers(server_sets: &[&[url::Url]]) -> Vec<url::Url> {
    let mut merged = Vec::new();
    for servers in server_sets {
        for server in *servers {
            push_unique(&mut merged, server.clone());
        }
    }
    merged
}

/// Uploads to each Blossom server and succeeds once at least one upload completes.
/// Returns the hash and all confirmed server URLs (primary first, then mirrors that succeeded).
pub async fn upload_to_blossom_servers(
    client: &dyn FileStorageClientApi,
    servers: &[url::Url],
    bytes: Vec<u8>,
    signer: &BcrKeys,
) -> Result<(Sha256HexHash, Vec<url::Url>)> {
    let (_, hash, confirmed_servers) =
        upload_to_blossom_servers_with_server(client, servers, bytes, signer).await?;
    Ok((hash, confirmed_servers))
}

pub async fn upload_to_blossom_servers_with_server(
    client: &dyn FileStorageClientApi,
    servers: &[url::Url],
    bytes: Vec<u8>,
    signer: &BcrKeys,
) -> Result<(url::Url, Sha256HexHash, Vec<url::Url>)> {
    if servers.is_empty() {
        return Err(Error::NotFound);
    }

    let mut source_success = None;
    let mut remaining_servers = Vec::new();
    let mut last_error = None;

    for server in servers {
        if source_success.is_none() {
            match client.upload(server, bytes.clone()).await {
                Ok(hash) => {
                    source_success = Some((server.clone(), hash));
                }
                Err(err) => {
                    warn!("Failed Blossom source upload to {server}: {err}");
                    last_error = Some(err);
                    remaining_servers.push(server.clone());
                }
            }
        } else {
            remaining_servers.push(server.clone());
        }
    }

    let Some((source_server, source_hash)) = source_success else {
        return match last_error {
            Some(err) => Err(err.into()),
            None => Err(Error::NotFound),
        };
    };

    let source_url = to_url(&source_server, &source_hash.to_string())?;
    let mut confirmed_servers = vec![source_server.clone()];

    for server in remaining_servers {
        match client
            .mirror(&server, &source_url, &source_hash, signer)
            .await
        {
            Ok(_) => {
                confirmed_servers.push(server.clone());
            }
            Err(mirror_err) => {
                warn!("Failed Blossom mirror to {server}: {mirror_err}");
                match client.upload(&server, bytes.clone()).await {
                    Ok(_) => {
                        confirmed_servers.push(server.clone());
                    }
                    Err(upload_err) => {
                        warn!("Failed Blossom fallback upload to {server}: {upload_err}");
                    }
                }
            }
        }
    }

    Ok((source_server, source_hash, confirmed_servers))
}

/// Downloads from the first Blossom server that returns the requested blob.
pub async fn download_from_blossom_servers(
    client: &dyn FileStorageClientApi,
    servers: &[url::Url],
    nostr_hash: &Sha256HexHash,
) -> Result<Vec<u8>> {
    if servers.is_empty() {
        return Err(Error::NotFound);
    }

    let mut last_error = None;
    for server in servers {
        match client.download(server, nostr_hash).await {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                warn!("Failed Blossom download from {server}: {err}");
                last_error = Some(err);
            }
        }
    }

    match last_error {
        Some(err) => Err(err.into()),
        None => Err(Error::NotFound),
    }
}

/// Downloads a file with fallback resolution and on-demand legacy enrichment.
/// Resolution order:
/// 1. Configured Blossom servers
/// 2. File-reference stored servers
/// 3. On-demand metadata enrichment (lazy - only when needed)
/// 4. Download retry using servers discovered during enrichment
pub async fn download_file_with_fallback(
    client: &dyn FileStorageClientApi,
    file_reference_store: Option<&Arc<dyn FileReferenceStoreApi>>,
    transport: Option<&Arc<dyn crate::service::transport_service::TransportServiceApi>>,
    configured_servers: &[url::Url],
    file_hash: &Sha256Hash,
    nostr_hash: &Sha256HexHash,
) -> Result<Vec<u8>> {
    // Try 1: Configured servers first
    if !configured_servers.is_empty() {
        debug!(
            "Trying download from {} configured servers for file {}",
            configured_servers.len(),
            file_hash
        );
        match download_from_blossom_servers(client, configured_servers, nostr_hash).await {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                debug!(
                    "Configured servers failed for file {}: {}, trying fallbacks",
                    file_hash, err
                );
            }
        }
    }

    // Try 2: File-reference stored servers
    if let Some(store) = file_reference_store {
        match store.get(file_hash).await {
            Ok(Some(file_ref)) if !file_ref.server_urls.is_empty() => {
                debug!(
                    "Trying download from {} file-reference servers for file {}",
                    file_ref.server_urls.len(),
                    file_hash
                );
                match download_from_blossom_servers(client, &file_ref.server_urls, nostr_hash).await
                {
                    Ok(bytes) => return Ok(bytes),
                    Err(err) => {
                        debug!(
                            "File-reference servers failed for file {}: {}, will try enrichment",
                            file_hash, err
                        );
                    }
                }
            }
            Ok(Some(_)) => {
                debug!(
                    "File reference exists but has no servers for file {}",
                    file_hash
                );
            }
            Ok(None) => {
                debug!("No file reference found for file {}", file_hash);
            }
            Err(e) => {
                debug!("Failed to get file reference for file {}: {}", file_hash, e);
            }
        }
    }

    // Try 3: On-demand metadata enrichment (lazy - only when earlier servers failed)
    // Only for important files that have a file reference entry
    let discovered_servers = if let (Some(store), Some(tr)) = (file_reference_store, transport) {
        match store.get(file_hash).await {
            Ok(Some(file_ref)) if file_ref.is_important => {
                debug!(
                    "Attempting on-demand metadata enrichment for file {}",
                    file_hash
                );
                match enrich_servers_from_metadata(tr, file_hash, nostr_hash).await {
                    Ok(servers) if !servers.is_empty() => {
                        debug!(
                            "Discovered {} servers from metadata for file {}",
                            servers.len(),
                            file_hash
                        );

                        // Persist discovered servers to file reference
                        match store.add_server_urls(file_hash, servers.clone()).await {
                            Ok(added) => {
                                if added {
                                    debug!(
                                        "Persisted {} new server URLs to file reference for file {}",
                                        servers.len(),
                                        file_hash
                                    );
                                } else {
                                    debug!(
                                        "All discovered servers already existed for file {}",
                                        file_hash
                                    );
                                }
                            }
                            Err(e) => {
                                debug!(
                                    "Failed to persist discovered servers for file {}: {}",
                                    file_hash, e
                                );
                            }
                        }

                        servers
                    }
                    Ok(_) => {
                        debug!("No servers discovered from metadata for file {}", file_hash);
                        vec![]
                    }
                    Err(e) => {
                        debug!("Metadata enrichment failed for file {}: {}", file_hash, e);
                        vec![]
                    }
                }
            }
            Ok(Some(_)) => {
                debug!(
                    "Skipping on-demand metadata enrichment for non-important file {}",
                    file_hash
                );
                vec![]
            }
            Ok(None) => vec![],
            Err(e) => {
                debug!(
                    "Failed to get file reference for metadata enrichment for file {}: {}",
                    file_hash, e
                );
                vec![]
            }
        }
    } else {
        vec![]
    };

    // Retry with discovered servers
    if !discovered_servers.is_empty() {
        debug!(
            "Trying download from {} discovered servers for file {}",
            discovered_servers.len(),
            file_hash
        );
        match download_from_blossom_servers(client, &discovered_servers, nostr_hash).await {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                debug!("Discovered servers failed for file {}: {}", file_hash, err);
            }
        }
    }

    Err(Error::NotFound)
}

/// Queries historical metadata events (kind:1063) to discover server URLs for a file.
async fn enrich_servers_from_metadata(
    transport: &Arc<dyn crate::service::transport_service::TransportServiceApi>,
    file_hash: &Sha256Hash,
    nostr_hash: &Sha256HexHash,
) -> Result<Vec<url::Url>> {
    let events = transport
        .query_file_metadata_events(&file_hash.to_string(), &nostr_hash.to_string())
        .await?;

    let mut urls = Vec::new();
    for event in events {
        for tag in event.tags.iter() {
            let tag_slice: Vec<&str> = tag.as_slice().iter().map(|s: &String| s.as_str()).collect();
            match tag_slice.as_slice() {
                ["url", url_str] => {
                    if let Ok(url) = url::Url::parse(url_str)
                        && is_valid_file_url(&url)
                    {
                        push_unique(&mut urls, url);
                    }
                }
                ["fallback", url_str] => {
                    if let Ok(url) = url::Url::parse(url_str)
                        && is_valid_file_url(&url)
                    {
                        push_unique(&mut urls, url);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(urls)
}

fn is_valid_file_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::file_storage::{Error as FileStorageError, MockFileStorageClientApi};
    use bcr_ebill_core::protocol::crypto::BcrKeys;
    use bitcoin::hashes::Hash;
    use mockall::predicate::eq;
    use nostr::event::FinalizeEvent;
    use std::str::FromStr;

    fn test_config() -> NostrConfig {
        NostrConfig {
            only_known_contacts: false,
            relays: vec![url::Url::parse("wss://relay.example.com").unwrap()],
            blossom_servers: vec![],
            max_relays: Some(50),
            relay_ack_threshold: 1,
        }
    }

    #[test]
    fn blossom_server_from_relay_converts_websocket_schemes() {
        assert_eq!(
            blossom_server_from_relay(&url::Url::parse("ws://relay.example.com").unwrap())
                .unwrap()
                .as_str(),
            "http://relay.example.com/"
        );
        assert_eq!(
            blossom_server_from_relay(&url::Url::parse("wss://relay.example.com").unwrap())
                .unwrap()
                .as_str(),
            "https://relay.example.com/"
        );
    }

    #[test]
    fn configured_and_resolved_blossom_servers_prefer_explicit_values() {
        let explicit = url::Url::parse("https://blossom.example.com").unwrap();
        let mut config = test_config();
        config.blossom_servers = vec![explicit.clone()];

        assert_eq!(configured_blossom_servers(&config), vec![explicit.clone()]);
        assert_eq!(
            resolve_blossom_servers(std::slice::from_ref(&explicit), &config.relays),
            vec![explicit]
        );
    }

    #[test]
    fn configured_and_resolved_blossom_servers_fallback_to_first_relay() {
        let config = test_config();
        let expected = url::Url::parse("https://relay.example.com/").unwrap();

        assert_eq!(configured_blossom_servers(&config), vec![expected.clone()]);
        assert_eq!(resolve_blossom_servers(&[], &config.relays), vec![expected]);
    }

    #[test]
    fn merge_blossom_servers_preserves_order_and_deduplicates() {
        let merged = merge_blossom_servers(&[
            &[
                url::Url::parse("https://one.example.com").unwrap(),
                url::Url::parse("https://two.example.com").unwrap(),
            ],
            &[
                url::Url::parse("https://two.example.com").unwrap(),
                url::Url::parse("https://three.example.com").unwrap(),
            ],
        ]);

        assert_eq!(
            merged,
            vec![
                url::Url::parse("https://one.example.com").unwrap(),
                url::Url::parse("https://two.example.com").unwrap(),
                url::Url::parse("https://three.example.com").unwrap(),
            ]
        );
    }

    #[tokio::test]
    async fn upload_to_blossom_servers_retries_failed_targets_with_mirror_then_direct_upload() {
        let mut client = MockFileStorageClientApi::new();
        let first = url::Url::parse("https://one.example.com").unwrap();
        let second = url::Url::parse("https://two.example.com").unwrap();
        let bytes = b"hello".to_vec();
        let signer = BcrKeys::new();
        let expected = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let source_url = to_url(&second, &expected.to_string()).unwrap();

        client
            .expect_upload()
            .with(eq(first.clone()), eq(bytes.clone()))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();
        client
            .expect_upload()
            .with(eq(second.clone()), eq(bytes.clone()))
            .returning(move |_, _| Ok(expected))
            .once();
        client
            .expect_mirror()
            .with(
                eq(first.clone()),
                eq(source_url),
                eq(expected),
                eq(signer.clone()),
            )
            .returning(|_, _, _, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();
        client
            .expect_upload()
            .with(eq(first.clone()), eq(bytes.clone()))
            .returning(move |_, _| Ok(expected))
            .once();

        let result = upload_to_blossom_servers(&client, &[first, second], bytes, &signer)
            .await
            .unwrap();

        assert_eq!(result.0, expected);
        assert_eq!(result.1.len(), 2);
    }

    #[tokio::test]
    async fn upload_to_blossom_servers_with_server_returns_source_server_after_mirroring() {
        let mut client = MockFileStorageClientApi::new();
        let first = url::Url::parse("https://one.example.com").unwrap();
        let second = url::Url::parse("https://two.example.com").unwrap();
        let bytes = b"hello".to_vec();
        let signer = BcrKeys::new();
        let expected = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let source_url = to_url(&first, &expected.to_string()).unwrap();

        client
            .expect_upload()
            .with(eq(first.clone()), eq(bytes.clone()))
            .returning(move |_, _| Ok(expected))
            .once();
        client
            .expect_mirror()
            .with(
                eq(second.clone()),
                eq(source_url),
                eq(expected),
                eq(signer.clone()),
            )
            .returning(move |_, _, _, _| Ok(expected))
            .once();

        let result = upload_to_blossom_servers_with_server(
            &client,
            &[first.clone(), second.clone()],
            bytes,
            &signer,
        )
        .await
        .unwrap();

        assert_eq!(result.0, first);
        assert_eq!(result.1, expected);
        assert_eq!(result.2.len(), 2);
    }

    #[tokio::test]
    async fn download_from_blossom_servers_falls_back_until_one_server_returns_data() {
        let mut client = MockFileStorageClientApi::new();
        let first = url::Url::parse("https://one.example.com").unwrap();
        let second = url::Url::parse("https://two.example.com").unwrap();
        let nostr_hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let expected = b"hello".to_vec();

        client
            .expect_download()
            .with(eq(first.clone()), eq(nostr_hash))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();
        let expected_clone = expected.clone();
        client
            .expect_download()
            .with(eq(second.clone()), eq(nostr_hash))
            .returning(move |_, _| Ok(expected_clone.clone()))
            .once();

        let result = download_from_blossom_servers(&client, &[first, second], &nostr_hash)
            .await
            .unwrap();

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn file_resolution_fallback_configured_servers_succeed_no_enrichment() {
        use crate::service::transport_service::MockTransportServiceApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;

        let mut client = MockFileStorageClientApi::new();
        let configured_server = url::Url::parse("https://configured.example.com").unwrap();
        let file_hash = Sha256Hash::from_bytes(b"test_hash");
        let nostr_hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let expected_bytes = b"file content".to_vec();

        // Configured server succeeds immediately
        let bytes_clone = expected_bytes.clone();
        client
            .expect_download()
            .with(eq(configured_server.clone()), eq(nostr_hash))
            .returning(move |_, _| Ok(bytes_clone.clone()))
            .once();

        let file_ref_store: Arc<dyn FileReferenceStoreApi> =
            Arc::new(MockFileReferenceStoreApiMock::new());
        let transport: Arc<dyn crate::service::transport_service::TransportServiceApi> =
            Arc::new(MockTransportServiceApi::new());

        let result = download_file_with_fallback(
            &client,
            Some(&file_ref_store),
            Some(&transport),
            &[configured_server],
            &file_hash,
            &nostr_hash,
        )
        .await;

        assert_eq!(result.unwrap(), expected_bytes);
    }

    #[tokio::test]
    async fn file_resolution_fallback_file_reference_servers_succeed_after_configured_fail() {
        use crate::service::transport_service::MockTransportServiceApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;
        use bcr_ebill_core::protocol::Name;
        use bcr_ebill_core::protocol::file_reference::FileReference;

        let mut client = MockFileStorageClientApi::new();
        let configured_server = url::Url::parse("https://configured.example.com").unwrap();
        let discovered_server = url::Url::parse("https://discovered.example.com").unwrap();
        let file_hash = Sha256Hash::from_bytes(b"test_hash");
        let nostr_hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let expected_bytes = b"file content".to_vec();

        // Configured server fails
        client
            .expect_download()
            .with(eq(configured_server.clone()), eq(nostr_hash))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();

        // File reference exists but has no servers
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut file_ref = FileReference::new(
            file_hash.clone(),
            bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
            Some(Name::new("test.txt").unwrap()),
        );
        file_ref.is_important = true;
        file_ref.server_urls = vec![];
        file_reference_store
            .expect_get()
            .with(eq(file_hash.clone()))
            .returning(move |_| Ok(Some(file_ref.clone())))
            .times(2); // Called once for Try 2, once before enrichment

        // Discovered server succeeds on retry
        let bytes_clone = expected_bytes.clone();
        client
            .expect_download()
            .with(eq(discovered_server.clone()), eq(nostr_hash))
            .returning(move |_, _| Ok(bytes_clone.clone()))
            .once();

        // Expect add_server_urls to be called with discovered servers
        file_reference_store
            .expect_add_server_urls()
            .with(eq(file_hash.clone()), eq(vec![discovered_server.clone()]))
            .returning(|_, _| Ok(true))
            .once();

        // Create a mock metadata event with the discovered server URL
        let keys = nostr::key::Keys::generate();
        let mut event_builder =
            nostr::event::EventBuilder::new(nostr::event::Kind::Custom(1063), "");
        event_builder =
            event_builder
                .tags([nostr::event::Tag::parse(vec!["url", discovered_server.as_ref()]).unwrap()]);
        let metadata_event = event_builder.finalize(&keys).expect("to sign event");

        let mut transport = MockTransportServiceApi::new();
        transport
            .expect_query_file_metadata_events()
            .with(eq(file_hash.to_string()), eq(nostr_hash.to_string()))
            .returning(move |_, _| Ok(vec![metadata_event.clone()]))
            .once();

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn crate::service::transport_service::TransportServiceApi> =
            Arc::new(transport);

        let result = download_file_with_fallback(
            &client,
            Some(&file_ref_store),
            Some(&transport_arc),
            &[configured_server],
            &file_hash,
            &nostr_hash,
        )
        .await;

        assert_eq!(result.unwrap(), expected_bytes);
    }

    #[tokio::test]
    async fn file_resolution_fallback_discovered_servers_persisted_without_duplicates() {
        use crate::service::transport_service::MockTransportServiceApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;
        use bcr_ebill_core::protocol::Name;
        use bcr_ebill_core::protocol::file_reference::FileReference;

        let mut client = MockFileStorageClientApi::new();
        let configured_server = url::Url::parse("https://configured.example.com").unwrap();
        let existing_server = url::Url::parse("https://existing.example.com").unwrap();
        let discovered_server = url::Url::parse("https://discovered.example.com").unwrap();
        let file_hash = Sha256Hash::from_bytes(b"test_hash");
        let nostr_hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();
        let expected_bytes = b"file content".to_vec();

        // Configured server fails
        client
            .expect_download()
            .with(eq(configured_server.clone()), eq(nostr_hash))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();

        // File reference exists with existing server, which also fails
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut file_ref = FileReference::new(
            file_hash.clone(),
            bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
            Some(Name::new("test.txt").unwrap()),
        );
        file_ref.is_important = true;
        file_ref.server_urls = vec![existing_server.clone()];
        file_reference_store
            .expect_get()
            .with(eq(file_hash.clone()))
            .returning(move |_| Ok(Some(file_ref.clone())))
            .times(2);

        // Existing server fails
        client
            .expect_download()
            .with(eq(existing_server.clone()), eq(nostr_hash))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();

        // Discovered server succeeds
        let bytes_clone = expected_bytes.clone();
        client
            .expect_download()
            .with(eq(discovered_server.clone()), eq(nostr_hash))
            .returning(move |_, _| Ok(bytes_clone.clone()))
            .once();

        // Expect add_server_urls - should only add the new discovered server, not the existing one
        file_reference_store
            .expect_add_server_urls()
            .with(eq(file_hash.clone()), eq(vec![discovered_server.clone()]))
            .returning(|_, _| Ok(true))
            .once();

        // Create metadata event with discovered server
        let keys = nostr::key::Keys::generate();
        let mut event_builder =
            nostr::event::EventBuilder::new(nostr::event::Kind::Custom(1063), "");
        event_builder =
            event_builder
                .tags([nostr::event::Tag::parse(vec!["url", discovered_server.as_ref()]).unwrap()]);
        let metadata_event = event_builder.finalize(&keys).expect("to sign event");

        let mut transport = MockTransportServiceApi::new();
        transport
            .expect_query_file_metadata_events()
            .with(eq(file_hash.to_string()), eq(nostr_hash.to_string()))
            .returning(move |_, _| Ok(vec![metadata_event.clone()]))
            .once();

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn crate::service::transport_service::TransportServiceApi> =
            Arc::new(transport);

        let result = download_file_with_fallback(
            &client,
            Some(&file_ref_store),
            Some(&transport_arc),
            &[configured_server],
            &file_hash,
            &nostr_hash,
        )
        .await;

        assert_eq!(result.unwrap(), expected_bytes);
    }

    #[tokio::test]
    async fn file_resolution_fallback_no_file_reference_skips_enrichment() {
        use crate::service::transport_service::MockTransportServiceApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;

        let mut client = MockFileStorageClientApi::new();
        let configured_server = url::Url::parse("https://configured.example.com").unwrap();
        let file_hash = Sha256Hash::from_bytes(b"test_hash");
        let nostr_hash = Sha256HexHash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap();

        // Configured server fails
        client
            .expect_download()
            .with(eq(configured_server.clone()), eq(nostr_hash))
            .returning(|_, _| Err(FileStorageError::InvalidRelayUrl.into()))
            .once();

        // No file reference exists - no enrichment should occur
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        file_reference_store
            .expect_get()
            .with(eq(file_hash.clone()))
            .returning(move |_| Ok(None))
            .times(2); // Try 2 (empty servers) and check before enrichment

        // Transport should NOT be called for metadata query
        let transport = MockTransportServiceApi::new();

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn crate::service::transport_service::TransportServiceApi> =
            Arc::new(transport);

        let result = download_file_with_fallback(
            &client,
            Some(&file_ref_store),
            Some(&transport_arc),
            &[configured_server],
            &file_hash,
            &nostr_hash,
        )
        .await;

        // Should fail because no servers work and no enrichment happens
        assert!(matches!(result, Err(crate::service::Error::NotFound)));
    }
}
