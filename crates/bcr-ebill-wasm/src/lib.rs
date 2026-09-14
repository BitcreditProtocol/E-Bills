#![recursion_limit = "256"]
#![allow(clippy::arc_with_non_send_sync)]
use api::general::VERSION;
use bcr_common::core::NodeId;
use bcr_ebill_api::constants::DEFAULT_INITIAL_SUBSCRIPTION_DELAY_SECONDS;
use bcr_ebill_api::util::validate_node_id_network;
use bcr_ebill_api::{Config as ApiConfig, MintConfig, NostrConfig, get_db_context, init};
use bcr_ebill_api::{CourtConfig, DevModeConfig, PaymentConfig};
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_persistence::SurrealDbConfig;
use context::{Context, get_ctx};
use job::run_jobs;
use log::{debug, info, warn};
use nostr::nips::nip19::ToBech32;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use std::thread_local;
use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    str::FromStr,
};
use tokio_with_wasm::alias as tokio;
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use crate::error::{JsErrorData, WasmError};

pub mod api;
mod context;
mod data;
mod error;
mod job;

/// Can deserialize from either a string or a vec of strings to allow backward compatibility with
/// single url config.
fn deserialize_esplora_urls<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Single(String),
        Multiple(Vec<String>),
    }

    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::Single(s) => Ok(vec![s]),
        StringOrVec::Multiple(v) => Ok(v),
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct Config {
    pub log_level: Option<String>,
    pub bitcoin_network: String,
    #[serde(
        alias = "esplora_base_url",
        deserialize_with = "deserialize_esplora_urls"
    )]
    pub esplora_base_urls: Vec<String>,
    pub nostr_relays: Vec<String>,
    pub blossom_servers: Option<Vec<String>>,
    pub nostr_only_known_contacts: Option<bool>,
    pub nostr_max_relays: Option<usize>,
    pub nostr_relay_ack_threshold: Option<usize>,
    pub job_runner_initial_delay_seconds: u32,
    pub job_runner_check_interval_seconds: u32,
    pub transport_initial_subscription_delay_seconds: Option<u32>,
    pub default_mint_url: String,
    pub default_mint_node_id: String,
    pub num_confirmations_for_payment: usize,
    pub dev_mode: bool,
    pub mandatory_email_confirmations: bool,
    pub default_court_url: String,
}

pub type Result<T> = std::result::Result<T, error::WasmError>;

/// Result type for the TypeScript API
/// export type TSResult<T> = { Success: T } | { Error: JsErrorData };
/// To check if it's an error, just check `TSResult.Error` if it's not set, it's a `TSResult.Success`
/// even if `TSResult.Success` has `undefined` as a value.
#[derive(Tsify, Debug, Clone, Serialize)]
pub enum TSResult<T> {
    Success(T),
    Error(JsErrorData),
}

impl<T> TSResult<T>
where
    T: Serialize,
{
    pub fn err(e: WasmError) -> Self {
        TSResult::Error(JsErrorData::from(e))
    }

    pub fn to_js(&self) -> JsValue {
        serde_wasm_bindgen::to_value(&self).expect("can serialize TSResult")
    }

    pub fn res_to_js(res: Result<T>) -> JsValue {
        match res {
            Ok(v) => Self::Success(v).to_js(),
            Err(e) => Self::err(e).to_js(),
        }
    }
}

thread_local! {
    static CONTEXT: RefCell<Option<&'static Context>> = const { RefCell::new(None) } ;
    static TRANSPORT_CONNECTED: Cell<bool> = const { Cell::new(false) };
    static LAST_CONTACT_PUBLISH_CHECK: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// Minimum time between contact data publish checks to avoid excessive network calls
/// in flaky network conditions. Set to 5 minutes.
const CONTACT_PUBLISH_CHECK_INTERVAL: Duration = Duration::from_secs(3600);

pub(crate) fn is_transport_connected() -> bool {
    TRANSPORT_CONNECTED.with(Cell::get)
}

pub(crate) fn set_transport_connected(connected: bool) {
    TRANSPORT_CONNECTED.with(|status| status.set(connected));
}

/// Ensures contact data is published to Nostr, with rate limiting to avoid excessive
/// network calls during flaky network conditions.
async fn ensure_transport_contact_data_published(ctx: &Context, default_mint_node_id: &NodeId) {
    // Check if we've already done a publish check recently
    let now = Duration::from_secs(Timestamp::now().inner());
    let should_skip = LAST_CONTACT_PUBLISH_CHECK.with(|last| {
        if let Some(last_check) = last.get()
            && now.saturating_sub(last_check) < CONTACT_PUBLISH_CHECK_INTERVAL
        {
            debug!("Skipping contact data publish check - last check was recent");
            return true;
        }
        last.set(Some(now));
        false
    });

    if should_skip {
        return;
    }

    if let Ok(full_identity) = ctx.identity_service.get_full_identity().await
        && let Err(e) = ctx
            .identity_service
            .publish_contact(&full_identity.identity, &full_identity.key_pair)
            .await
    {
        warn!("Could not publish identity details to Nostr: {e}")
    }

    if let Ok(companies) = ctx.company_service.get_list_of_companies().await {
        for c in companies.iter() {
            if let Ok((company, keys)) = ctx.company_service.get_company_and_keys_by_id(&c.id).await
                && let Err(e) = ctx.company_service.publish_contact(&company, &keys).await
            {
                warn!("Could not publish company details to Nostr: {e}")
            }
        }
    }

    ctx.transport_service
        .contact_transport()
        .ensure_nostr_contact(default_mint_node_id)
        .await;
}

#[wasm_bindgen]
pub async fn initialize_api(
    #[wasm_bindgen(unchecked_param_type = "Config")] cfg: JsValue,
) -> Result<()> {
    // init config and API
    let config: Config = serde_wasm_bindgen::from_value(cfg)?;

    // init logging
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let log_level = match config.log_level {
        Some(ref log_level) => match log_level.as_str() {
            "info" => log::LevelFilter::Info,
            "debug" => log::LevelFilter::Debug,
            "error" => log::LevelFilter::Error,
            "trace" => log::LevelFilter::Trace,
            _ => log::LevelFilter::Info,
        },
        None => log::LevelFilter::Info,
    };
    // only log from our own crates
    fern::Dispatch::new()
        .level(log::LevelFilter::Off)
        .level_for("bcr_ebill_wasm", log_level)
        .level_for("bcr_ebill_api", log_level)
        .level_for("bcr_ebill_persistence", log_level)
        .level_for("bcr_ebill_transport", log_level)
        .level_for("bcr_ebill_core", log_level)
        .chain(fern::Output::call(console_log::log))
        .apply()
        .expect("can initialize logging");
    let nostr_relays: Vec<url::Url> = config
        .nostr_relays
        .iter()
        .map(|nr| url::Url::parse(nr).expect("nostr relay is not a valid URL"))
        .collect();
    let blossom_servers: Vec<url::Url> = config
        .blossom_servers
        .unwrap_or_default()
        .iter()
        .map(|server| url::Url::parse(server).expect("blossom server is not a valid URL"))
        .collect();
    let mint_node_id = NodeId::from_str(&config.default_mint_node_id).expect("is a valid mint id");
    let api_config = ApiConfig {
        bitcoin_network: config.bitcoin_network,
        esplora_base_urls: config
            .esplora_base_urls
            .iter()
            .map(|u| url::Url::parse(u).expect("esplora base url is not a valid URL"))
            .collect(),
        db_config: SurrealDbConfig::default(), // unused in WASM builds
        files_db_config: SurrealDbConfig::default(), // unused in WASM builds
        nostr_config: NostrConfig {
            relays: nostr_relays,
            blossom_servers,
            only_known_contacts: config.nostr_only_known_contacts.unwrap_or(false),
            max_relays: config.nostr_max_relays.or(Some(50)),
            relay_ack_threshold: config.nostr_relay_ack_threshold.unwrap_or(1),
        },
        mint_config: MintConfig::new(config.default_mint_url, mint_node_id)?,
        payment_config: PaymentConfig {
            num_confirmations_for_payment: config.num_confirmations_for_payment,
        },
        dev_mode_config: DevModeConfig {
            on: config.dev_mode,
            mandatory_email_confirmations: config.mandatory_email_confirmations,
        },
        court_config: CourtConfig {
            default_url: url::Url::parse(&config.default_court_url)
                .expect("court url is not a valid URL"),
        },
    };
    debug!("Config: {api_config:?}");
    init(api_config.clone())?;
    // make sure the configured default mint node id is valid for the configured network
    validate_node_id_network(&api_config.mint_config.default_mint_node_id)?;

    // init db
    let db = get_db_context(&api_config).await?;
    // set the network and check if the configured network matches the persisted network and fail, if not
    db.identity_store
        .set_or_check_network(api_config.bitcoin_network())
        .await?;
    let keys = db.identity_store.get_or_create_key_pair().await?;

    let node_id = NodeId::new(keys.pub_key(), api_config.bitcoin_network());
    info!("Initialized WASM API {VERSION}");
    info!("Local node id: {node_id}");
    info!(
        "Local npub: {}",
        node_id
            .npub()
            .to_bech32()
            .expect("invalid npub from node id")
    );
    info!("Local npub as hex: {}", node_id.npub().to_hex());

    // init context as static reference
    let ctx = Context::new(api_config.clone(), db).await?;
    CONTEXT.with(|context| {
        let mut context_ref = context.borrow_mut();
        if context_ref.is_none() {
            let leaked: &'static Context = Box::leak(Box::new(ctx)); // leak to get a static ref
            *context_ref = Some(leaked);
        }
    });

    // start jobs
    wasm_bindgen_futures::spawn_local(async move {
        tokio::time::sleep(Duration::from_secs(
            config.job_runner_initial_delay_seconds as u64,
        ))
        .await;

        // before first run we ensure if we have a connection to the transports
        // as there could be jobs that require a connection
        get_ctx().transport_service.connect().await;

        run_jobs(); // initial run
        let mut interval = tokio::time::interval(Duration::from_secs(
            config.job_runner_check_interval_seconds as u64,
        ));
        loop {
            interval.tick().await;
            run_jobs(); // regular run
        }
    });

    let default_mint_node_id = api_config.mint_config.default_mint_node_id.clone();
    // start nostr subscription
    wasm_bindgen_futures::spawn_local(async move {
        tokio::time::sleep(Duration::from_secs(
            config
                .transport_initial_subscription_delay_seconds
                .unwrap_or(DEFAULT_INITIAL_SUBSCRIPTION_DELAY_SECONDS) as u64,
        ))
        .await;

        let reconnect_interval_seconds = config.job_runner_check_interval_seconds as u64;
        set_transport_connected(false);

        loop {
            let ctx = get_ctx();

            info!("Connecting to Nostr transport..");
            // before subscription we ensure if we have a connection to the transports
            ctx.transport_service.connect().await;

            ensure_transport_contact_data_published(ctx, &default_mint_node_id).await;

            match ctx.nostr_consumer.start().await {
                Ok(mut handle) => {
                    set_transport_connected(true);
                    info!("Nostr transport connected");
                    while let Some(result) = handle.join_next().await {
                        match result {
                            Ok(()) => info!("Nostr consumer task shutdown with success"),
                            Err(e) => warn!("Nostr consumer task shutdown with error: {e}"),
                        }
                    }
                    warn!("Nostr consumer stopped, reconnecting...");
                }
                Err(e) => {
                    warn!("Could not start Nostr consumer: {e}");
                }
            }

            set_transport_connected(false);
            tokio::time::sleep(Duration::from_secs(reconnect_interval_seconds)).await;
        }
    });
    Ok(())
}

// nostr uses universal-time, which doesn't define a clock for WASM
#[cfg(target_arch = "wasm32")]
mod wasm_time {
    use core::time::Duration;
    use universal_time::{Instant, MonotonicClock, SystemTime, WallClock, define_time_provider};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn performance_now() -> f64;
    }

    struct WasmTimeProvider;

    impl WallClock for WasmTimeProvider {
        fn system_time(&self) -> SystemTime {
            SystemTime::from_unix_duration(Duration::from_secs_f64(js_sys::Date::now() / 1000.0))
        }
    }

    impl MonotonicClock for WasmTimeProvider {
        fn instant(&self) -> Instant {
            Instant::from_ticks(Duration::from_secs_f64(performance_now() / 1000.0))
        }
    }

    define_time_provider!(WasmTimeProvider);
}
