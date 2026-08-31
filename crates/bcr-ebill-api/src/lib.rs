#![recursion_limit = "256"]
use anyhow::{Result, anyhow};
use bcr_common::core::NodeId;
use bcr_ebill_persistence::db::surreal::SurrealWrapper;
#[cfg(not(target_arch = "wasm32"))]
use bcr_ebill_persistence::get_surreal_db;
use bcr_ebill_persistence::{
    ContactStoreApi, FileReferenceStoreApi, NostrChainEventStoreApi, NostrContactStoreApi,
    NostrEventOffsetStoreApi, NotificationStoreApi, SurrealBillChainStore, SurrealBillStore,
    SurrealCompanyChainStore, SurrealCompanyStore, SurrealContactStore, SurrealDbConfig,
    SurrealFileReferenceStore, SurrealIdentityChainStore, SurrealIdentityStore,
    SurrealNostrChainEventStore, SurrealNostrContactStore, SurrealNostrEventOffsetStore,
    SurrealNotificationStore,
    bill::{BillChainStoreApi, BillStoreApi},
    company::{CompanyChainStoreApi, CompanyStoreApi},
    db::{
        email_notification::SurrealEmailNotificationStore, mint::SurrealMintStore,
        nostr_send_queue::SurrealNostrEventQueueStore,
    },
    file_upload::FileUploadStoreApi,
    identity::{IdentityChainStoreApi, IdentityStoreApi},
    mint::MintStoreApi,
    nostr::NostrQueuedMessageStoreApi,
    notification::EmailNotificationStoreApi,
};
use bitcoin::Network;
use log::error;
use std::sync::{Arc, OnceLock};

pub mod constants;
pub mod external;
pub mod service;
#[cfg(test)]
mod tests;
pub mod util;

#[derive(Debug, Clone)]
pub struct Config {
    pub bitcoin_network: String,
    /// List of Esplora API base URLs (in order of priority).
    /// The first URL is used for API requests with fallback to subsequent URLs on failure.
    /// The first URL is also used for user-facing links (e.g., mempool explorer links).
    pub esplora_base_urls: Vec<url::Url>,
    pub db_config: SurrealDbConfig,
    pub files_db_config: SurrealDbConfig,
    pub nostr_config: NostrConfig,
    pub mint_config: MintConfig,
    pub payment_config: PaymentConfig,
    pub dev_mode_config: DevModeConfig,
    pub court_config: CourtConfig,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

impl Config {
    pub fn bitcoin_network(&self) -> Network {
        self.try_bitcoin_network()
            .expect("bitcoin network must be validated during API initialization")
    }

    fn try_bitcoin_network(&self) -> Result<Network> {
        Self::try_bitcoin_network_value(&self.bitcoin_network)
    }

    fn try_bitcoin_network_value(configured: &str) -> Result<Network> {
        match configured {
            "mainnet" | "bitcoin" => Ok(Network::Bitcoin),
            "testnet" => Ok(Network::Testnet),
            "testnet4" => Ok(Network::Testnet4),
            "regtest" => Ok(Network::Regtest),
            unsupported => Err(anyhow!(
                "Unsupported bitcoin network '{unsupported}'; expected bitcoin, mainnet, testnet, testnet4, or regtest"
            )),
        }
    }
}

#[cfg(test)]
mod config_network_tests {
    use super::Config;
    use bitcoin::Network;

    #[test]
    fn parses_every_supported_bitcoin_network_explicitly() {
        let cases = [
            ("bitcoin", Network::Bitcoin),
            ("mainnet", Network::Bitcoin),
            ("testnet", Network::Testnet),
            ("testnet4", Network::Testnet4),
            ("regtest", Network::Regtest),
        ];

        for (configured, expected) in cases {
            let parsed = Config::try_bitcoin_network_value(configured).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn rejects_unknown_bitcoin_network_instead_of_falling_back() {
        let error = Config::try_bitcoin_network_value("testnett").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsupported bitcoin network 'testnett'")
        );
    }
}

/// Court specific configuration
#[derive(Debug, Clone)]
pub struct CourtConfig {
    /// The default court URL
    pub default_url: url::Url,
}

/// Developer Mode specific configuration
#[derive(Debug, Clone)]
pub struct DevModeConfig {
    /// Whether dev mode is on
    pub on: bool,
    /// Whether mandatory email confirmations should be enabled (disable for easier testing)
    pub mandatory_email_confirmations: bool,
}

/// Payment specific configuration
#[derive(Debug, Clone, Default)]
pub struct PaymentConfig {
    /// Amount of confirmations until we consider an on-chain payment as paid
    pub num_confirmations_for_payment: usize,
}

/// Nostr specific configuration
#[derive(Debug, Clone)]
pub struct NostrConfig {
    /// Only known contacts can message us via DM.
    pub only_known_contacts: bool,
    /// All relays we want to publish our messages to and receive messages from.
    pub relays: Vec<url::Url>,
    /// Blossom servers we want to publish and use for file storage.
    pub blossom_servers: Vec<url::Url>,
    /// Maximum number of contact relays to add (in addition to user relays which are always included).
    /// Defaults to 50 if not specified.
    pub max_relays: Option<usize>,
    /// Number of relay acknowledgements required before an optimistic broadcast
    /// returns to the caller. The remaining relays continue publishing in the background.
    pub relay_ack_threshold: usize,
}

impl Default for NostrConfig {
    fn default() -> Self {
        Self {
            only_known_contacts: false,
            relays: vec![],
            blossom_servers: vec![],
            max_relays: Some(50),
            relay_ack_threshold: 1,
        }
    }
}

/// Mint configuration
#[derive(Debug, Clone)]
pub struct MintConfig {
    /// URL of the default mint
    pub default_mint_url: url::Url,
    /// Node Id of the default mint
    pub default_mint_node_id: NodeId,
}

impl MintConfig {
    pub fn new(default_mint_url: String, default_mint_node_id: NodeId) -> Result<Self> {
        let url = url::Url::parse(&default_mint_url)
            .map_err(|e| anyhow!("Invalid Default Mint URL: {e}"))?;
        Ok(Self {
            default_mint_url: url,
            default_mint_node_id,
        })
    }
}

pub fn init(conf: Config) -> Result<()> {
    if conf.esplora_base_urls.is_empty() {
        return Err(anyhow!("esplora_base_urls must contain at least one URL"));
    }
    conf.try_bitcoin_network()?;

    CONFIG
        .set(conf)
        .map_err(|e| anyhow!("Could not initialize E-Bill API: {e:?}"))?;
    Ok(())
}

pub fn get_config() -> &'static Config {
    CONFIG.get().expect("E-Bill API is not initialized")
}

/// A container for all persistence related dependencies.
#[derive(Clone)]
pub struct DbContext {
    pub contact_store: Arc<dyn ContactStoreApi>,
    pub bill_store: Arc<dyn BillStoreApi>,
    pub bill_blockchain_store: Arc<dyn BillChainStoreApi>,
    pub identity_store: Arc<dyn IdentityStoreApi>,
    pub identity_chain_store: Arc<dyn IdentityChainStoreApi>,
    pub company_chain_store: Arc<dyn CompanyChainStoreApi>,
    pub company_store: Arc<dyn CompanyStoreApi>,
    pub file_upload_store: Arc<dyn FileUploadStoreApi>,
    pub file_reference_store: Arc<dyn FileReferenceStoreApi>,
    pub nostr_event_offset_store: Arc<dyn NostrEventOffsetStoreApi>,
    pub notification_store: Arc<dyn NotificationStoreApi>,
    pub email_notification_store: Arc<dyn EmailNotificationStoreApi>,
    pub queued_message_store: Arc<dyn NostrQueuedMessageStoreApi>,
    pub nostr_contact_store: Arc<dyn NostrContactStoreApi>,
    pub mint_store: Arc<dyn MintStoreApi>,
    pub nostr_chain_event_store: Arc<dyn NostrChainEventStoreApi>,
}

/// Creates a new instance of the DbContext with the given SurrealDB configuration.
pub async fn get_db_context(
    #[allow(unused)] conf: &Config,
) -> bcr_ebill_persistence::Result<DbContext> {
    #[cfg(not(target_arch = "wasm32"))]
    let db = get_surreal_db(&conf.db_config).await?;
    #[cfg(not(target_arch = "wasm32"))]
    let files_db = get_surreal_db(&conf.files_db_config).await?;
    #[cfg(not(target_arch = "wasm32"))]
    let surreal_wrapper = SurrealWrapper {
        db: db.clone(),
        files: false,
    };

    #[cfg(not(target_arch = "wasm32"))]
    let files_surreal_wrapper = SurrealWrapper {
        db: files_db.clone(),
        files: true,
    };

    #[cfg(target_arch = "wasm32")]
    let surreal_wrapper = SurrealWrapper { files: false };

    #[cfg(target_arch = "wasm32")]
    let files_surreal_wrapper = SurrealWrapper { files: true };

    let company_store = Arc::new(SurrealCompanyStore::new(surreal_wrapper.clone()));
    let file_upload_store = Arc::new(
        bcr_ebill_persistence::db::file_upload::FileUploadStore::new(files_surreal_wrapper),
    );

    if let Err(e) = file_upload_store.cleanup_temp_uploads().await {
        error!("Error cleaning up temp uploads: {e}");
    }

    let contact_store = Arc::new(SurrealContactStore::new(surreal_wrapper.clone()));

    let bill_store = Arc::new(SurrealBillStore::new(surreal_wrapper.clone()));
    let bill_blockchain_store = Arc::new(SurrealBillChainStore::new(surreal_wrapper.clone()));

    let identity_store = Arc::new(SurrealIdentityStore::new(surreal_wrapper.clone()));
    let identity_chain_store = Arc::new(SurrealIdentityChainStore::new(surreal_wrapper.clone()));
    let company_chain_store = Arc::new(SurrealCompanyChainStore::new(surreal_wrapper.clone()));

    let nostr_event_offset_store =
        Arc::new(SurrealNostrEventOffsetStore::new(surreal_wrapper.clone()));
    let notification_store = Arc::new(SurrealNotificationStore::new(surreal_wrapper.clone()));
    let email_notification_store =
        Arc::new(SurrealEmailNotificationStore::new(surreal_wrapper.clone()));

    let queued_message_store = Arc::new(SurrealNostrEventQueueStore::new(surreal_wrapper.clone()));
    let nostr_contact_store = Arc::new(SurrealNostrContactStore::new(surreal_wrapper.clone()));
    let mint_store = Arc::new(SurrealMintStore::new(surreal_wrapper.clone()));
    let nostr_chain_event_store =
        Arc::new(SurrealNostrChainEventStore::new(surreal_wrapper.clone()));
    let file_reference_store = Arc::new(SurrealFileReferenceStore::new(surreal_wrapper.clone()));

    Ok(DbContext {
        contact_store,
        bill_store,
        bill_blockchain_store,
        identity_store,
        identity_chain_store,
        company_chain_store,
        company_store,
        file_upload_store,
        file_reference_store,
        nostr_event_offset_store,
        notification_store,
        email_notification_store,
        queued_message_store,
        nostr_contact_store,
        mint_store,
        nostr_chain_event_store,
    })
}
