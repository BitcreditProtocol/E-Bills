use crate::external::file_storage::FileStorageClientApi;
use crate::service::transport_service::TransportServiceApi;
use crate::service::{Error, Result, file_server_service::upload_to_blossom_servers};
use bcr_common::core::NodeId;
use bcr_ebill_core::protocol::{
    File, Name, PublicKey, Sha256Hash,
    crypto::{self, BcrKeys},
    file_reference::FileReferenceContext,
};
use bcr_ebill_persistence::{Error as PersistenceError, FileReferenceStoreApi};
use log::{debug, info};
use std::collections::HashSet;
use std::sync::Arc;

pub async fn encrypt_upload_and_track_file(
    file_reference_store: &Arc<dyn FileReferenceStoreApi>,
    file_storage_client: &Arc<dyn FileStorageClientApi>,
    transport: &Arc<dyn TransportServiceApi>,
    configured_servers: &[url::Url],
    file_name: &Name,
    file_bytes: &[u8],
    public_key: &PublicKey,
    signer: &BcrKeys,
    node_id: &NodeId,
    context: FileReferenceContext,
    mime_type: Option<String>,
    owner_label: &str,
) -> Result<File> {
    let file_hash = Sha256Hash::from_bytes(file_bytes);
    let encrypted = crypto::encrypt_ecies(file_bytes, public_key)?;
    let (nostr_hash, confirmed_servers) = upload_to_blossom_servers(
        file_storage_client.as_ref(),
        configured_servers,
        encrypted,
        signer,
    )
    .await?;

    info!("Saved {owner_label} file {file_name} with hash {file_hash} for node {node_id}");

    let file = File {
        name: file_name.to_owned(),
        hash: file_hash,
        nostr_hash,
    };

    upsert_important_file_reference(file_reference_store, &file, context.clone(), vec![]).await?;

    record_confirmed_servers_and_publish(
        file_reference_store,
        transport,
        node_id,
        &file,
        confirmed_servers,
        mime_type,
        true,
    )
    .await?;

    if let Err(e) = enforce_important_file_replication(
        file_reference_store,
        file_storage_client,
        transport,
        configured_servers,
        &file,
        context,
        node_id,
    )
    .await
    {
        debug!("Replication enforcement failed after upload: {e}");
    }

    Ok(file)
}

pub async fn enforce_important_file_replication(
    file_reference_store: &Arc<dyn FileReferenceStoreApi>,
    file_storage_client: &Arc<dyn FileStorageClientApi>,
    transport: &Arc<dyn TransportServiceApi>,
    configured_servers: &[url::Url],
    file: &File,
    context: FileReferenceContext,
    node_id: &NodeId,
) -> Result<bool> {
    let file_ref = match file_reference_store.get(&file.hash).await? {
        Some(ref_file) => {
            if !ref_file.is_important {
                return Ok(false);
            }
            ref_file
        }
        None => {
            file_reference_store
                .upsert(
                    &file.hash,
                    &file.nostr_hash,
                    Some(file.name.clone()),
                    vec![],
                    Some(true),
                    vec![context.clone()],
                )
                .await?;
            file_reference_store.get(&file.hash).await?.ok_or_else(|| {
                Error::Persistence(PersistenceError::NoSuchEntity(
                    "file reference".to_string(),
                    file.hash.to_string(),
                ))
            })?
        }
    };

    let missing_servers: Vec<url::Url> = configured_servers
        .iter()
        .filter(|configured| {
            !file_ref
                .server_urls
                .iter()
                .any(|confirmed| urls_equal(configured, confirmed))
        })
        .cloned()
        .collect();

    if missing_servers.is_empty() {
        return Ok(false);
    }

    debug!(
        "Replicating file {} to {} missing configured server(s)",
        file.hash,
        missing_servers.len()
    );

    let encrypted_bytes = match download_file_bytes(
        file_storage_client.as_ref(),
        &file_ref.server_urls,
        &file.nostr_hash,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(Error::NotFound);
        }
    };

    let mut confirmed_servers = Vec::new();
    for server in &missing_servers {
        if replicate_file_to_server(file_storage_client.as_ref(), server, &encrypted_bytes)
            .await
            .is_ok()
        {
            confirmed_servers.push(server.clone());
        }
    }

    if confirmed_servers.is_empty() {
        return Ok(false);
    }

    record_confirmed_servers_and_publish(
        file_reference_store,
        transport,
        node_id,
        file,
        confirmed_servers,
        Some("application/octet-stream".to_string()),
        false,
    )
    .await?;

    Ok(true)
}

/// Downloads encrypted file bytes from available servers.
async fn download_file_bytes(
    client: &dyn FileStorageClientApi,
    servers: &[url::Url],
    nostr_hash: &bitcoin::hashes::sha256::Hash,
) -> Result<Vec<u8>> {
    for server in servers {
        if let Ok(bytes) = client.download(server, nostr_hash).await {
            return Ok(bytes);
        }
    }
    Err(Error::NotFound)
}

async fn replicate_file_to_server(
    client: &dyn FileStorageClientApi,
    server: &url::Url,
    encrypted_bytes: &[u8],
) -> Result<()> {
    client
        .upload(server, encrypted_bytes.to_vec())
        .await
        .map(|_| ())
        .map_err(|e| e.into())
}

fn urls_equal(a: &url::Url, b: &url::Url) -> bool {
    let a_norm = normalize_url(a);
    let b_norm = normalize_url(b);
    a_norm == b_norm
}

fn normalize_url(url: &url::Url) -> String {
    let mut s = url.to_string();
    if s.ends_with('/') {
        s.pop();
    }
    s.to_lowercase()
}

pub async fn record_confirmed_servers_and_publish(
    file_reference_store: &Arc<dyn FileReferenceStoreApi>,
    transport: &Arc<dyn TransportServiceApi>,
    node_id: &NodeId,
    file: &File,
    confirmed_servers: Vec<url::Url>,
    mime_type: Option<String>,
    force_publish: bool,
) -> Result<()> {
    if confirmed_servers.is_empty() {
        debug!(
            "Skipping server recording - no confirmed servers for file {}",
            file.hash
        );
        return Ok(());
    }

    let should_publish = match file_reference_store.get(&file.hash).await? {
        Some(existing) => {
            let existing_set = normalized_url_set(&existing.server_urls);
            let confirmed_set = normalized_url_set(&confirmed_servers);
            !confirmed_set.is_subset(&existing_set)
        }
        None => {
            // No existing record, should publish
            true
        }
    };

    // Add the confirmed servers to the file reference
    let added = file_reference_store
        .add_server_urls(&file.hash, confirmed_servers.clone())
        .await?;

    if force_publish || should_publish || added {
        debug!(
            "Publishing kind:1063 metadata for file {} with {} confirmed servers",
            file.hash,
            confirmed_servers.len()
        );

        // Publish the file metadata event
        transport
            .publish_file_metadata(
                node_id,
                &file.hash.to_string(),
                &file.nostr_hash.to_string(),
                confirmed_servers,
                mime_type,
            )
            .await?;
    } else {
        debug!(
            "Skipping kind:1063 publish for file {} - no new servers to announce",
            file.hash
        );
    }

    Ok(())
}

pub async fn upsert_important_file_reference(
    file_reference_store: &Arc<dyn FileReferenceStoreApi>,
    file: &File,
    context: FileReferenceContext,
    server_urls: Vec<url::Url>,
) -> crate::service::Result<()> {
    debug!(
        "Upserting important file reference for file {} with context {:?}",
        file.hash, context
    );

    file_reference_store
        .upsert(
            &file.hash,
            &file.nostr_hash,
            Some(file.name.clone()),
            server_urls,
            Some(true),
            vec![context],
        )
        .await?;

    Ok(())
}

fn normalized_url_set(urls: &[url::Url]) -> HashSet<String> {
    urls.iter().map(normalize_url).collect()
}

pub fn identity_file_context(field: &str) -> FileReferenceContext {
    FileReferenceContext::Identity {
        field: field.to_string(),
    }
}

pub fn company_file_context(company_id: &NodeId, field: &str) -> FileReferenceContext {
    FileReferenceContext::Company {
        company_id: company_id.to_string(),
        field: field.to_string(),
    }
}

pub fn contact_file_context(node_id: &NodeId, field: &str) -> FileReferenceContext {
    FileReferenceContext::Contact {
        node_id: node_id.to_string(),
        field: field.to_string(),
    }
}

pub fn bill_file_context(bill_id: &str, field: &str) -> FileReferenceContext {
    FileReferenceContext::Bill {
        bill_id: bill_id.to_string(),
        field: field.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{
        COMPANY_LOGO_FILE_FIELD, IDENTITY_DOCUMENT_FILE_FIELD, IDENTITY_PROFILE_PICTURE_FILE_FIELD,
    };
    use crate::tests::tests::MockFileReferenceStoreApiMock;
    use bcr_ebill_core::protocol::{Name, file_reference::FileReference};
    use bitcoin::hashes::Hash;
    use mockall::predicate;
    use std::str::FromStr;

    mockall::mock! {
        TransportService {}

        #[async_trait::async_trait]
        impl TransportServiceApi for TransportService {
            fn block_transport(&self) -> &std::sync::Arc<dyn crate::service::transport_service::BlockTransportServiceApi>;
            fn contact_transport(&self) -> &std::sync::Arc<dyn crate::service::transport_service::ContactTransportServiceApi>;
            fn notification_transport(&self) -> &std::sync::Arc<dyn crate::service::transport_service::NotificationTransportServiceApi>;
            async fn connect(&self);
            async fn send_bill_is_signed_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent) -> crate::service::transport_service::Result<()>;
            async fn send_bill_is_accepted_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent) -> crate::service::transport_service::Result<()>;
            async fn send_request_to_accept_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent) -> crate::service::transport_service::Result<()>;
            async fn send_request_to_pay_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent) -> crate::service::transport_service::Result<()>;
            async fn send_bill_is_endorsed_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent) -> crate::service::transport_service::Result<()>;
            async fn send_offer_to_sell_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent, buyer: &bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant) -> crate::service::transport_service::Result<()>;
            async fn send_bill_is_sold_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent, buyer: &bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant) -> crate::service::transport_service::Result<()>;
            async fn send_bill_recourse_paid_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent, recoursee: &bcr_ebill_core::protocol::blockchain::bill::participant::BillIdentParticipant) -> crate::service::transport_service::Result<()>;
            async fn send_request_to_mint_event(&self, sender_node_id: &bcr_common::core::NodeId, mint: &bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant, bill: &bcr_ebill_core::protocol::blockchain::bill::BitcreditBill) -> crate::service::transport_service::Result<()>;
            async fn send_request_to_action_rejected_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent, rejected_action: bcr_ebill_core::protocol::event::ActionType) -> crate::service::transport_service::Result<()>;
            async fn send_recourse_action_event(&self, event: &bcr_ebill_core::protocol::event::BillChainEvent, action: bcr_ebill_core::protocol::event::ActionType, recoursee: &bcr_ebill_core::protocol::blockchain::bill::participant::BillIdentParticipant) -> crate::service::transport_service::Result<()>;
            async fn send_retry_messages(&self) -> crate::service::transport_service::Result<()>;
            async fn sync_relays(&self) -> crate::service::transport_service::Result<()>;
            async fn retry_failed_syncs(&self) -> crate::service::transport_service::Result<()>;
            async fn add_identity(&self, node_id: &bcr_common::core::NodeId, keys: &bcr_ebill_core::protocol::crypto::BcrKeys) -> crate::service::transport_service::Result<()>;
            async fn process_company_historical_bill_invites(&self, company_id: &bcr_common::core::NodeId) -> crate::service::transport_service::Result<()>;
            async fn publish_file_metadata(&self, node_id: &bcr_common::core::NodeId, plaintext_hash: &str, encrypted_hash: &str, server_urls: Vec<url::Url>, mime_type: Option<String>) -> crate::service::transport_service::Result<()>;
            async fn query_file_metadata_events(&self, file_hash: &str, nostr_hash: &str) -> crate::service::transport_service::Result<Vec<nostr::event::Event>>;
        }
    }

    impl bcr_ebill_core::application::ServiceTraitBounds for MockTransportService {}

    #[tokio::test]
    async fn test_file_metadata_publish_skipped_when_no_confirmed_servers() {
        let file_reference_store: Arc<dyn FileReferenceStoreApi> =
            Arc::new(MockFileReferenceStoreApiMock::new());
        let transport: Arc<dyn TransportServiceApi> = Arc::new(MockTransportService::new());
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let result = record_confirmed_servers_and_publish(
            &file_reference_store,
            &transport,
            &node_id,
            &file,
            vec![],
            None,
            false,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_file_metadata_publish_with_confirmed_servers() {
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let confirmed_servers = vec![url::Url::parse("https://blossom1.example.com").unwrap()];

        file_reference_store.expect_get().returning(|_| Ok(None));

        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(true));

        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()))
            .times(1);

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let result = record_confirmed_servers_and_publish(
            &file_ref_store,
            &transport_arc,
            &node_id,
            &file,
            confirmed_servers,
            Some("text/plain".to_string()),
            false,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_file_metadata_publish_idempotent_same_servers() {
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let confirmed_servers = vec![url::Url::parse("https://blossom1.example.com").unwrap()];

        let mut existing_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        existing_ref.server_urls = confirmed_servers.clone();
        existing_ref.is_important = true;

        file_reference_store
            .expect_get()
            .returning(move |_| Ok(Some(existing_ref.clone())));

        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(false));

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let result = record_confirmed_servers_and_publish(
            &file_ref_store,
            &transport_arc,
            &node_id,
            &file,
            confirmed_servers,
            None,
            false,
        )
        .await;

        assert!(result.is_ok());
    }

    #[test]
    fn test_identity_file_context() {
        let ctx = identity_file_context("avatar_file");
        assert_eq!(
            ctx,
            FileReferenceContext::Identity {
                field: "avatar_file".to_string()
            }
        );
    }

    #[test]
    fn test_company_file_context() {
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let ctx = company_file_context(&node_id, COMPANY_LOGO_FILE_FIELD);
        assert_eq!(
            ctx,
            FileReferenceContext::Company {
                company_id: node_id.to_string(),
                field: COMPANY_LOGO_FILE_FIELD.to_string()
            }
        );
    }

    #[test]
    fn test_contact_file_context() {
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let ctx = contact_file_context(&node_id, "avatar_file");
        assert_eq!(
            ctx,
            FileReferenceContext::Contact {
                node_id: node_id.to_string(),
                field: "avatar_file".to_string()
            }
        );
    }

    #[test]
    fn test_bill_file_context() {
        let ctx = bill_file_context("bill123", "attachment_0");
        assert_eq!(
            ctx,
            FileReferenceContext::Bill {
                bill_id: "bill123".to_string(),
                field: "attachment_0".to_string()
            }
        );
    }

    fn test_helper_file() -> File {
        File {
            name: Name::new("test_file.txt").unwrap(),
            hash: Sha256Hash::new("test_hash_12345678901234567890123456789012"),
            nostr_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .parse()
                .unwrap(),
        }
    }

    fn test_helper_node_id() -> NodeId {
        NodeId::from_str("bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0")
            .unwrap()
    }

    fn create_upsert_mock_store() -> Arc<dyn FileReferenceStoreApi> {
        let mut mock_store = MockFileReferenceStoreApiMock::new();

        mock_store
            .expect_upsert()
            .with(
                predicate::always(),
                predicate::always(),
                predicate::always(),
                predicate::always(),
                predicate::eq(Some(true)),
                predicate::always(),
            )
            .returning(|hash, nostr_hash, name, _, _, _| {
                Ok(FileReference::new(hash.clone(), *nostr_hash, name))
            });

        Arc::new(mock_store)
    }

    fn test_helper_server_urls() -> Vec<url::Url> {
        vec![url::Url::parse("https://blossom.example.com").unwrap()]
    }

    #[tokio::test]
    async fn important_file_upsert_identity_file() {
        let file = test_helper_file();
        let mock_store = create_upsert_mock_store();

        let result = upsert_important_file_reference(
            &mock_store,
            &file,
            identity_file_context(IDENTITY_PROFILE_PICTURE_FILE_FIELD),
            test_helper_server_urls(),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn important_file_upsert_company_file() {
        let file = test_helper_file();
        let company_id = test_helper_node_id();
        let mock_store = create_upsert_mock_store();

        let result = upsert_important_file_reference(
            &mock_store,
            &file,
            company_file_context(&company_id, COMPANY_LOGO_FILE_FIELD),
            test_helper_server_urls(),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn important_file_upsert_contact_file() {
        let file = test_helper_file();
        let node_id = test_helper_node_id();
        let mock_store = create_upsert_mock_store();

        let result = upsert_important_file_reference(
            &mock_store,
            &file,
            contact_file_context(&node_id, "avatar_file"),
            test_helper_server_urls(),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn important_file_upsert_bill_file() {
        let file = test_helper_file();
        let mock_store = create_upsert_mock_store();

        let result = upsert_important_file_reference(
            &mock_store,
            &file,
            bill_file_context("bill123", "attachment_0"),
            test_helper_server_urls(),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn important_file_upsert_marks_as_important() {
        let file = test_helper_file();
        let mut mock_store = MockFileReferenceStoreApiMock::new();

        mock_store
            .expect_upsert()
            .with(
                predicate::always(),
                predicate::always(),
                predicate::always(),
                predicate::always(),
                predicate::eq(Some(true)),
                predicate::always(),
            )
            .returning(|hash, nostr_hash, name, _, _, _| {
                Ok(FileReference::new(hash.clone(), *nostr_hash, name))
            })
            .once();

        let mock_store: Arc<dyn FileReferenceStoreApi> = Arc::new(mock_store);

        let result = upsert_important_file_reference(
            &mock_store,
            &file,
            identity_file_context(IDENTITY_DOCUMENT_FILE_FIELD),
            test_helper_server_urls(),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_file_metadata_publish_forced_on_seeded_servers() {
        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let confirmed_servers = vec![url::Url::parse("https://blossom1.example.com").unwrap()];
        let mut existing_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        existing_ref.server_urls = confirmed_servers.clone();
        existing_ref.is_important = true;

        file_reference_store
            .expect_get()
            .returning(move |_| Ok(Some(existing_ref.clone())));
        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(false));
        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()))
            .once();

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let result = record_confirmed_servers_and_publish(
            &file_ref_store,
            &transport_arc,
            &node_id,
            &file,
            confirmed_servers,
            None,
            true,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn important_file_replication_skips_non_important_file() {
        use crate::external::file_storage::MockFileStorageClientApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;

        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let file_storage_client = MockFileStorageClientApi::new();
        let transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let mut file_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        file_ref.is_important = false;

        file_reference_store
            .expect_get()
            .returning(move |_| Ok(Some(file_ref.clone())));

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let file_storage_arc: Arc<dyn FileStorageClientApi> = Arc::new(file_storage_client);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let configured_servers = vec![url::Url::parse("https://blossom1.example.com").unwrap()];

        let result = enforce_important_file_replication(
            &file_ref_store,
            &file_storage_arc,
            &transport_arc,
            &configured_servers,
            &file,
            identity_file_context("test_file"),
            &node_id,
        )
        .await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn important_file_replication_triggers_when_missing_configured_server() {
        use crate::external::file_storage::MockFileStorageClientApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;
        use bitcoin::hashes::Hash;

        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut file_storage_client = MockFileStorageClientApi::new();
        let mut transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let existing_server = url::Url::parse("https://existing.example.com").unwrap();
        let missing_server = url::Url::parse("https://missing.example.com").unwrap();

        let mut file_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        file_ref.is_important = true;
        file_ref.server_urls = vec![existing_server.clone()];

        file_reference_store
            .expect_get()
            .returning(move |_| Ok(Some(file_ref.clone())));

        let existing_server_clone = existing_server.clone();
        file_storage_client
            .expect_download()
            .withf(move |url, _| url.as_str() == existing_server_clone.as_str())
            .returning(|_, _| Ok(vec![1, 2, 3, 4, 5]));

        file_storage_client
            .expect_upload()
            .returning(|_, _| Ok(bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap()));

        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(true));

        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()));

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let file_storage_arc: Arc<dyn FileStorageClientApi> = Arc::new(file_storage_client);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let configured_servers = vec![existing_server.clone(), missing_server.clone()];

        let result = enforce_important_file_replication(
            &file_ref_store,
            &file_storage_arc,
            &transport_arc,
            &configured_servers,
            &file,
            identity_file_context("test_file"),
            &node_id,
        )
        .await;

        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn important_file_replication_is_idempotent_when_all_servers_covered() {
        use crate::external::file_storage::MockFileStorageClientApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;

        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let file_storage_client = MockFileStorageClientApi::new();
        let transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let server1 = url::Url::parse("https://blossom1.example.com").unwrap();
        let server2 = url::Url::parse("https://blossom2.example.com").unwrap();

        let mut file_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        file_ref.is_important = true;
        file_ref.server_urls = vec![server1.clone(), server2.clone()];

        file_reference_store
            .expect_get()
            .returning(move |_| Ok(Some(file_ref.clone())));

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let file_storage_arc: Arc<dyn FileStorageClientApi> = Arc::new(file_storage_client);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let configured_servers = vec![server1.clone(), server2.clone()];

        let result = enforce_important_file_replication(
            &file_ref_store,
            &file_storage_arc,
            &transport_arc,
            &configured_servers,
            &file,
            identity_file_context("test_file"),
            &node_id,
        )
        .await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn important_file_replication_creates_file_ref_if_missing() {
        use crate::external::file_storage::MockFileStorageClientApi;
        use crate::tests::tests::MockFileReferenceStoreApiMock;
        use bitcoin::hashes::Hash;

        let mut file_reference_store = MockFileReferenceStoreApiMock::new();
        let mut file_storage_client = MockFileStorageClientApi::new();
        let transport = MockTransportService::new();
        let node_id = NodeId::from_str(
            "bitcrt02295fb5f4eeb2f21e01eaf3a2d9a3be10f39db870d28f02146130317973a40ac0",
        )
        .unwrap();
        let file = File {
            name: Name::new("test.txt").unwrap(),
            hash: Sha256Hash::from_bytes(b"test"),
            nostr_hash: bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
        };

        let configured_server = url::Url::parse("https://blossom1.example.com").unwrap();

        file_reference_store.expect_get().returning(|_| Ok(None));

        file_reference_store
            .expect_upsert()
            .returning(|_, _, _, _, _, _| {
                Ok(FileReference::new(
                    Sha256Hash::from_bytes(b"test"),
                    bitcoin::hashes::sha256::Hash::from_slice(&[0u8; 32]).unwrap(),
                    None,
                ))
            });

        let mut file_ref =
            FileReference::new(file.hash.clone(), file.nostr_hash, Some(file.name.clone()));
        file_ref.is_important = true;

        file_storage_client.expect_download().returning(|_, _| {
            Err(crate::external::Error::ExternalFileStorageApi(
                crate::external::file_storage::Error::InvalidHash,
            ))
        });

        let file_ref_store: Arc<dyn FileReferenceStoreApi> = Arc::new(file_reference_store);
        let file_storage_arc: Arc<dyn FileStorageClientApi> = Arc::new(file_storage_client);
        let transport_arc: Arc<dyn TransportServiceApi> = Arc::new(transport);

        let configured_servers = vec![configured_server];

        let result = enforce_important_file_replication(
            &file_ref_store,
            &file_storage_arc,
            &transport_arc,
            &configured_servers,
            &file,
            identity_file_context("test_file"),
            &node_id,
        )
        .await;

        assert!(result.is_err());
    }
}
