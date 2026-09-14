use std::{collections::HashMap, str::FromStr};

use crate::get_config;
use async_trait::async_trait;
use bcr_ebill_core::{
    application::{
        ServiceTraitBounds,
        bill::{InMempoolData, PaidData, PaymentState, SweepEstimate, SweepOption, SweepResult},
    },
    protocol::{
        BitcoinAddress, Sum, Timestamp,
        crypto::btc::{BtcDescriptor, parse_private_descriptor},
    },
};
use bitcoin::secp256k1::{Keypair, Message, SECP256K1, SecretKey};
use bitcoin::{
    Amount, Network, OutPoint, ScriptBuf, Sequence, TapSighashType, Transaction, TxIn, TxOut, Txid,
    Witness,
    absolute::LockTime,
    consensus::encode::serialize_hex,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot, transaction,
};
use log::debug;
use log::warn;
use miniscript::ToPublicKey;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::alias::try_join;
use tokio_with_wasm as tokio;

pub const DUST_THRESHOLD: u64 = 546;
pub const FEE_ESTIMATE_DEFAULT: f64 = 1.0;

/// Generic result type
pub type Result<T> = std::result::Result<T, super::Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// all errors originating from interacting with the web api
    #[error("External Bitcoin Web API error: {0}")]
    Api(#[from] reqwest::Error),

    /// all errors originating from dealing with invalid data from the API
    #[error("Got invalid data from the API")]
    InvalidData(String),

    /// all errors originating from dealing with bitcoin descriptors
    #[error("External Bitcoin Descriptor error: {0}")]
    Descriptor(String),

    /// all errors originating when there are insufficient funds to sweep
    #[error("External Bitcoin InsufficientFunds error: available: {0}, needed: {1}")]
    InsufficientFunds(u64, u64),

    /// all errors originating when addresses are not valid
    #[error("External Bitcoin InvalidAddress error: {0}")]
    InvalidAddress(String),

    /// all errors originating from sending transactions with an amount that's below the dust amount
    #[error("Amount would be dust: {0} sat")]
    Dust(u64),
}

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait BitcoinClientApi: ServiceTraitBounds {
    /// Checks payment by iterating over the transactions on the address in chronological order, until
    /// the target amount is filled, returning the respective payment status
    async fn check_payment_for_address(
        &self,
        address: &BitcoinAddress,
        target_amount: u64,
    ) -> Result<PaymentState>;

    /// Generates a payment link
    fn generate_link_to_pay(&self, address: &BitcoinAddress, sum: &Sum, message: &str) -> String;

    /// Generates a link to check the address on the configured mempool browser
    fn get_mempool_link_for_address(&self, address: &BitcoinAddress) -> String;

    /// Checks and estimates fee for sweeping the funds to the destination address
    async fn check_and_estimate_sweep(
        &self,
        private_descriptor: &BtcDescriptor,
        destination_address: &BitcoinAddress,
    ) -> Result<SweepEstimate>;

    async fn sweep_funds(
        &self,
        private_descriptor: &BtcDescriptor,
        destination_address: &BitcoinAddress,
        fee_sat: u64,
    ) -> Result<SweepResult>;
}

#[derive(Clone)]
pub struct BitcoinClient {
    cl: reqwest::Client,
    esplora_base_urls: Vec<url::Url>,
    network: Network,
}

impl ServiceTraitBounds for BitcoinClient {}

#[cfg(test)]
impl ServiceTraitBounds for MockBitcoinClientApi {}

impl BitcoinClient {
    pub fn new() -> Self {
        Self::from_config(get_config())
    }

    pub fn from_config(config: &crate::Config) -> Self {
        Self {
            cl: reqwest::Client::new(),
            esplora_base_urls: config.esplora_base_urls.clone(),
            network: config.bitcoin_network(),
        }
    }

    #[cfg(test)]
    pub fn with_urls(esplora_base_urls: Vec<url::Url>, network: Network) -> Self {
        Self {
            cl: reqwest::Client::new(),
            esplora_base_urls,
            network,
        }
    }

    fn build_api_url(&self, base_url: &url::Url, path: &str) -> String {
        match self.network {
            Network::Bitcoin => format!("{}api{path}", base_url),
            Network::Regtest => format!("{}regtest/api{path}", base_url),
            _ => format!("{}testnet/api{path}", base_url),
        }
    }

    fn build_link_url(&self, base_url: &url::Url, path: &str) -> String {
        match self.network {
            Network::Bitcoin => format!("{}{path}", base_url),
            Network::Regtest => format!("{}regtest/{path}", base_url),
            _ => format!("{}testnet/{path}", base_url),
        }
    }

    fn primary_base_url(&self) -> &url::Url {
        self.esplora_base_urls
            .first()
            .expect("esplora_base_urls must not be empty")
    }

    pub fn link_url(&self, path: &str) -> String {
        self.build_link_url(self.primary_base_url(), path)
    }

    fn is_retryable_error(error: &reqwest::Error) -> bool {
        if let Some(status) = error.status() {
            return status.is_server_error()
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT;
        }
        true
    }

    async fn request_with_fallback<T, F, P, Fut>(
        &self,
        path_builder: F,
        ctx: ReqContext,
        parse: P,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn(&url::Url) -> String,
        P: Fn(reqwest::Response) -> Fut,
        Fut: Future<Output = std::result::Result<T, reqwest::Error>>,
    {
        let mut last_error: Option<Error> = None;

        for (i, base_url) in self.esplora_base_urls.iter().enumerate() {
            let url = path_builder(base_url);
            debug!(
                "Trying Esplora URL {}/{}: {}",
                i + 1,
                self.esplora_base_urls.len(),
                url
            );

            let call = match ctx {
                ReqContext::Get => self.cl.get(&url).send(),
                ReqContext::Post { ref payload } => self.cl.post(&url).body(payload.clone()).send(),
            };

            match call.await {
                Ok(response) => {
                    let status = response.status();

                    if status.is_server_error()
                        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || status == reqwest::StatusCode::REQUEST_TIMEOUT
                    {
                        warn!(
                            "Esplora URL {} returned retryable status {}, trying next",
                            base_url, status
                        );
                        last_error = Some(Error::InvalidData(format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("Unknown")
                        )));
                        continue;
                    }

                    match response.error_for_status() {
                        Ok(res) => return parse(res).await.map_err(|e| Error::Api(e).into()),
                        Err(e) => return Err(Error::Api(e).into()),
                    };
                }
                Err(e) => {
                    if Self::is_retryable_error(&e) && i + 1 < self.esplora_base_urls.len() {
                        warn!(
                            "Esplora URL {} failed with retryable error: {}, trying next",
                            base_url, e
                        );
                        last_error = Some(Error::Api(e));
                        continue;
                    }
                    return Err(Error::Api(e).into());
                }
            }
        }

        Err(last_error
            .expect("esplora_base_urls must not be empty")
            .into())
    }

    async fn get_transactions(&self, address: &BitcoinAddress) -> Result<Transactions> {
        let addr_str = address.assume_checked_ref().to_string();
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, &format!("/address/{addr_str}/txs")),
            ReqContext::Get,
            |response| async move { response.json::<Transactions>().await },
        )
        .await
    }

    async fn get_last_block_height(&self) -> Result<u64> {
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, "/blocks/tip/height"),
            ReqContext::Get,
            |response| async move { response.json::<u64>().await },
        )
        .await
    }

    async fn get_fee_estimates(&self) -> Result<HashMap<String, f64>> {
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, "/fee-estimates"),
            ReqContext::Get,
            |response| async move { response.json::<HashMap<String, f64>>().await },
        )
        .await
    }

    async fn get_utxos(&self, address: &BitcoinAddress) -> Result<Vec<Utxo>> {
        let addr = address.assume_checked_ref().to_string();
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, &format!("/address/{addr}/utxo")),
            ReqContext::Get,
            |response| async move { response.json::<Vec<Utxo>>().await },
        )
        .await
    }

    async fn get_tx(&self, txid: &Txid) -> Result<TxFull> {
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, &format!("/tx/{txid}")),
            ReqContext::Get,
            |response| async move { response.json::<TxFull>().await },
        )
        .await
    }

    async fn broadcast_tx(&self, tx_hex: &str) -> Result<String> {
        self.request_with_fallback(
            |base_url| self.build_api_url(base_url, "/tx"),
            ReqContext::Post {
                payload: tx_hex.to_owned(),
            },
            |response| async move { response.text().await },
        )
        .await
    }

    async fn spendable_utxos_and_amount(
        &self,
        private_key: &bitcoin::PrivateKey,
        utxos: &[Utxo],
    ) -> Result<(Vec<SpendableUtxo>, u64)> {
        let expected_script_pubkey = ScriptBuf::new_p2tr(
            SECP256K1,
            private_key.public_key(SECP256K1).to_x_only_pubkey(),
            None,
        );

        let mut spendables = Vec::with_capacity(utxos.len());
        let mut available_funds: u64 = 0;

        for u in utxos {
            // we only count confirmed utxos
            if !u.status.confirmed {
                continue;
            }

            let txid = Txid::from_str(&u.txid)
                .map_err(|e| Error::InvalidData(format!("invalid txid {}: {}", u.txid, e)))?;
            let tx = self.get_tx(&txid).await?;
            let vout = tx.vout.get(u.vout as usize).ok_or_else(|| {
                Error::InvalidData(format!("missing vout {} for {}", u.vout, txid))
            })?;

            let script_pubkey = ScriptBuf::from_hex(&vout.scriptpubkey)
                .map_err(|e| Error::InvalidData(format!("invalid scriptpubkey: {}", e)))?;

            // we only count p2tr outputs that belong to our key
            if script_pubkey != expected_script_pubkey {
                continue;
            }

            available_funds += vout.value;

            spendables.push(SpendableUtxo {
                outpoint: OutPoint { txid, vout: u.vout },
                prevout: TxOut {
                    value: Amount::from_sat(vout.value),
                    script_pubkey,
                },
            });
        }

        Ok((spendables, available_funds))
    }
}

impl Default for BitcoinClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl BitcoinClientApi for BitcoinClient {
    async fn check_payment_for_address(
        &self,
        address: &BitcoinAddress,
        target_amount: u64,
    ) -> Result<PaymentState> {
        debug!(
            "checking if btc address {} is paid {target_amount}",
            address.assume_checked_ref()
        );
        // in parallel, get current chain height and transactions
        let (chain_block_height, txs) =
            try_join!(self.get_last_block_height(), self.get_transactions(address))?;

        payment_state_from_transactions(chain_block_height, txs, address, target_amount)
    }

    fn generate_link_to_pay(&self, address: &BitcoinAddress, sum: &Sum, message: &str) -> String {
        let btc_sum = sum.as_btc_string();
        let link = format!(
            "bitcoin:{}?amount={btc_sum}&message={message}",
            address.assume_checked_ref()
        );
        link
    }

    fn get_mempool_link_for_address(&self, address: &BitcoinAddress) -> String {
        self.link_url(&format!("address/{}", address.assume_checked_ref()))
    }

    async fn check_and_estimate_sweep(
        &self,
        private_descriptor: &BtcDescriptor,
        destination_address: &BitcoinAddress,
    ) -> Result<SweepEstimate> {
        let btc_network = self.network;
        let (_desc, private_key, source_address) =
            parse_private_descriptor(private_descriptor, btc_network)
                .map_err(|e| Error::Descriptor(e.to_string()))?;

        if &source_address == destination_address {
            return Err(
                Error::InvalidAddress("Can't sweep to the same address".to_string()).into(),
            );
        }

        if !source_address.is_valid_for_network(btc_network) {
            return Err(
                Error::InvalidAddress("Wrong network for source address".to_string()).into(),
            );
        }

        if !destination_address.is_valid_for_network(btc_network) {
            return Err(
                Error::InvalidAddress("Wrong network for destination address".to_string()).into(),
            );
        }

        // sum up available funds
        let utxos = self.get_utxos(source_address.as_unchecked()).await?;
        let (spendable_utxos, available_funds) = self
            .spendable_utxos_and_amount(&private_key, &utxos)
            .await?;

        // funds already sweeped, or were never there
        if available_funds == 0 {
            return Ok(SweepEstimate {
                available_funds: 0,
                economy: SweepOption {
                    fee_rate_sat_vb: 0.0,
                    fee_sat: 0,
                    amount_to_sweep_sat: 0,
                },
                fast: SweepOption {
                    fee_rate_sat_vb: 0.0,
                    fee_sat: 0,
                    amount_to_sweep_sat: 0,
                },
            });
        }

        let mut draft_tx_for_estimation = build_unsigned_sweep_tx(
            &spendable_utxos,
            destination_address.assume_checked_ref(),
            available_funds,
        );

        // Add dummy witnesses for vsize estimation
        let mut dummy_witness = Witness::new();
        dummy_witness.push(vec![0u8; 64]);
        for input in draft_tx_for_estimation.input.iter_mut() {
            input.witness = dummy_witness.clone();
        }
        let vsize_estimate = draft_tx_for_estimation.vsize();

        let fee_estimates = self.get_fee_estimates().await?;
        // If we don't get a proper value for the fee, we use a default
        let (economic_fee_estimate, fast_fee_estimate) = (
            // for 6 blocks
            fee_estimates.get("6").unwrap_or_else(|| {
                log::warn!("Couldn't get fee estimate for 6 blocks - defaulting to 1.0");
                &FEE_ESTIMATE_DEFAULT
            }),
            // for 1 block
            fee_estimates.get("1").unwrap_or_else(|| {
                log::warn!("Couldn't get fee estimate for 1 block - defaulting to 1.0");
                &FEE_ESTIMATE_DEFAULT
            }),
        );

        let economy = create_sweep_option(available_funds, *economic_fee_estimate, vsize_estimate)?;
        let fast = create_sweep_option(available_funds, *fast_fee_estimate, vsize_estimate)?;

        Ok(SweepEstimate {
            available_funds,
            economy,
            fast,
        })
    }

    async fn sweep_funds(
        &self,
        private_descriptor: &BtcDescriptor,
        destination_address: &BitcoinAddress,
        fee_sat: u64,
    ) -> Result<SweepResult> {
        let btc_network = self.network;
        let (_desc, private_key, source_address) =
            parse_private_descriptor(private_descriptor, btc_network)
                .map_err(|e| Error::Descriptor(e.to_string()))?;

        if &source_address == destination_address {
            return Err(
                Error::InvalidAddress("Can't sweep to the same address".to_string()).into(),
            );
        }

        if !source_address.is_valid_for_network(btc_network) {
            return Err(
                Error::InvalidAddress("Wrong network for source address".to_string()).into(),
            );
        }

        if !destination_address.is_valid_for_network(btc_network) {
            return Err(
                Error::InvalidAddress("Wrong network for destination address".to_string()).into(),
            );
        }

        let utxos = self.get_utxos(source_address.as_unchecked()).await?;
        let (spendable_utxos, available_funds) = self
            .spendable_utxos_and_amount(&private_key, &utxos)
            .await?;

        // do checks for amount
        if available_funds == 0 {
            return Err(Error::InsufficientFunds(available_funds, 0).into());
        }

        let sweep_amount = available_funds
            .checked_sub(fee_sat)
            .ok_or(Error::InsufficientFunds(available_funds, fee_sat))?;

        if sweep_amount < DUST_THRESHOLD {
            return Err(Error::Dust(sweep_amount).into());
        }

        // create tx, collect prevouts and sign inputs
        let mut tx = build_unsigned_sweep_tx(
            &spendable_utxos,
            destination_address.assume_checked_ref(),
            sweep_amount,
        );

        let prevouts_vec: Vec<TxOut> = spendable_utxos.iter().map(|u| u.prevout.clone()).collect();
        let prevouts = Prevouts::All(&prevouts_vec);

        for i in 0..tx.input.len() {
            sign_p2tr_keypath_input(&mut tx, i, &prevouts, &private_key.inner)?;
        }

        // broadcast tx
        let tx_id = self.broadcast_tx(&serialize_hex(&tx)).await?;
        let link = self.link_url(&format!("tx/{}", tx_id));

        Ok(SweepResult {
            tx_id,
            link_to_tx: link,
            fee_sat,
            sweep_amount,
        })
    }
}

fn sign_p2tr_keypath_input(
    tx: &mut Transaction,
    input_idx: usize,
    prevouts: &Prevouts<'_, TxOut>,
    secret_key: &SecretKey,
) -> Result<()> {
    let keypair = Keypair::from_secret_key(SECP256K1, secret_key);
    let tweaked_keypair = keypair.tap_tweak(SECP256K1, None);

    let sighash = SighashCache::new(&mut *tx)
        .taproot_key_spend_signature_hash(input_idx, prevouts, TapSighashType::Default)
        .map_err(|e| Error::InvalidData(format!("taproot sighash computation failed: {e}")))?;

    let msg = Message::from_digest_slice(sighash.as_ref()).map_err(|e| {
        Error::InvalidData(format!(
            "invalid sighash message for schnorr signature: {e}"
        ))
    })?;
    let sig = SECP256K1.sign_schnorr(&msg, &tweaked_keypair.to_keypair());

    let mut witness = Witness::new();
    witness.push(
        taproot::Signature {
            signature: sig,
            sighash_type: TapSighashType::Default,
        }
        .to_vec(),
    );
    tx.input[input_idx].witness = witness;

    Ok(())
}

/// builds an unsigned tx for the given spendable utxos
fn build_unsigned_sweep_tx(
    utxos: &[SpendableUtxo],
    destination: &bitcoin::Address,
    output_sat: u64,
) -> Transaction {
    let input = utxos
        .iter()
        .map(|u| TxIn {
            previous_output: u.outpoint,
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    let output = vec![TxOut {
        value: Amount::from_sat(output_sat),
        script_pubkey: destination.script_pubkey(),
    }];

    Transaction {
        version: transaction::Version::TWO,
        lock_time: LockTime::ZERO,
        input,
        output,
    }
}

fn create_sweep_option(
    available_funds_sat: u64,
    fee_rate_sat_vb: f64,
    estimated_vsize: usize,
) -> Result<SweepOption> {
    let fee_sat = (fee_rate_sat_vb * estimated_vsize as f64).ceil() as u64;

    let amount_to_sweep_sat = available_funds_sat
        .checked_sub(fee_sat)
        .ok_or(Error::InsufficientFunds(available_funds_sat, fee_sat))?;

    if amount_to_sweep_sat < DUST_THRESHOLD {
        return Err(Error::Dust(amount_to_sweep_sat).into());
    }

    Ok(SweepOption {
        fee_rate_sat_vb,
        fee_sat,
        amount_to_sweep_sat,
    })
}

fn payment_state_from_transactions(
    chain_block_height: u64,
    txs: Transactions,
    address: &BitcoinAddress,
    target_amount: u64,
) -> Result<PaymentState> {
    // no transactions - no payment
    if txs.is_empty() {
        return Ok(PaymentState::NotFound);
    }
    let addr_string = address.assume_checked_ref().to_string();

    let mut total = 0;
    let mut tx_filled = None;

    // sort from back to front (chronologically)
    for tx in txs.iter().rev() {
        for vout in tx.vout.iter() {
            // sum up outputs towards the address to check
            if let Some(ref addr) = vout.scriptpubkey_address
                && addr == &addr_string
            {
                total += vout.value;
            }
        }
        // if the current transaction covers the amount, we save it and break
        if total >= target_amount {
            tx_filled = Some(tx.to_owned());
            break;
        }
    }

    match tx_filled {
        Some(tx) => {
            // in mem pool
            if !tx.status.confirmed {
                debug!("payment for {addr_string} is in mem pool {}", tx.txid);
                Ok(PaymentState::InMempool(InMempoolData { tx_id: tx.txid }))
            } else {
                match (
                    tx.status.block_height,
                    tx.status.block_time,
                    tx.status.block_hash,
                ) {
                    (Some(block_height), Some(block_time), Some(block_hash)) => {
                        let confirmations = chain_block_height - block_height + 1;
                        let paid_data = PaidData {
                            block_time: Timestamp::new(block_time).map_err(|_| Error::InvalidData(format!("Invalid data when checking payment for {addr_string} - invalid block time")))?,
                            block_hash,
                            confirmations,
                            tx_id: tx.txid,
                        };
                        if confirmations
                            >= get_config().payment_config.num_confirmations_for_payment as u64
                        {
                            // paid and confirmed
                            debug!(
                                "payment for {addr_string} is paid and confirmed with {confirmations} confirmations"
                            );
                            Ok(PaymentState::PaidConfirmed(paid_data))
                        } else {
                            // paid but not enough confirmations yet
                            debug!(
                                "payment for {addr_string} is paid and unconfirmed with {confirmations} confirmations"
                            );
                            Ok(PaymentState::PaidUnconfirmed(paid_data))
                        }
                    }
                    _ => {
                        log::error!(
                            "Invalid data when checking payment for {addr_string} - confirmed tx, but no metadata"
                        );
                        Err(Error::InvalidData(format!("Invalid data when checking payment for {addr_string} - confirmed tx, but no metadata")).into())
                    }
                }
            }
        }
        None => {
            // not enough funds to cover amount
            debug!(
                "Not enough funds to cover {target_amount} yet when checking payment for {addr_string}: {total}"
            );
            Ok(PaymentState::NotFound)
        }
    }
}

#[derive(Serialize, Debug, Clone)]
enum ReqContext {
    Get,
    Post { payload: String },
}

/// Fields documented at https://github.com/Blockstream/esplora/blob/master/API.md#addresses
#[derive(Deserialize, Debug, Clone)]
pub struct AddressInfo {
    pub chain_stats: Stats,
    pub mempool_stats: Stats,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Stats {
    pub funded_txo_sum: u64,
    pub spent_txo_sum: u64,
}

pub type Transactions = Vec<Tx>;

/// Available fields documented at https://github.com/Blockstream/esplora/blob/master/API.md#transactions
#[derive(Deserialize, Debug, Clone)]
pub struct Tx {
    pub txid: String,
    pub status: Status,
    pub vout: Vec<Vout>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Vout {
    pub value: u64,
    pub scriptpubkey_address: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Status {
    // Height of the block the tx is in
    pub block_height: Option<u64>,
    // Unix Timestamp
    pub block_time: Option<u64>,
    // Hash of the block the tx is in
    pub block_hash: Option<String>,
    // Whether it's in the mempool (false), or in a block (true)
    pub confirmed: bool,
}

#[derive(Debug, Clone)]
struct SpendableUtxo {
    outpoint: OutPoint,
    prevout: TxOut,
}

#[derive(Debug, Deserialize)]
struct Utxo {
    txid: String,
    vout: u32,
    status: UtxoStatus,
}

#[derive(Debug, Deserialize)]
struct UtxoStatus {
    confirmed: bool,
}

#[derive(Debug, Deserialize)]
struct TxFull {
    vout: Vec<TxVout>,
}

#[derive(Debug, Deserialize)]
struct TxVout {
    value: u64,
    scriptpubkey: String,
}

#[cfg(test)]
pub mod tests {
    use super::{BitcoinClient, Error, Result, Status, Tx, Vout, payment_state_from_transactions};
    use crate::{
        external::bitcoin::{BitcoinClientApi, ReqContext},
        tests::tests::init_test_cfg,
    };
    use std::str::FromStr;

    use bcr_ebill_core::{
        application::bill::PaymentState,
        protocol::{BitcoinAddress, crypto::btc::BtcDescriptor},
    };
    use bitcoin::Network;
    use mockito;
    use serde_json::json;

    #[tokio::test]
    async fn test_fallback_on_server_error() {
        let mut server1 = mockito::Server::new_async().await;
        let mut server2 = mockito::Server::new_async().await;

        let m1 = server1
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let m2 = server2
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(200)
            .with_body("12345")
            .expect(1)
            .create_async()
            .await;

        let client = BitcoinClient::with_urls(
            vec![
                url::Url::parse(&server1.url()).unwrap(),
                url::Url::parse(&server2.url()).unwrap(),
            ],
            Network::Testnet,
        );

        let result: Result<u64> = client
            .request_with_fallback(
                |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                ReqContext::Get,
                |response| async move { response.json::<u64>().await },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 12345);

        m1.assert_async().await;
        m2.assert_async().await;
    }

    #[ignore] // takes very long - activate on-demand
    #[tokio::test]
    async fn test_fallback_on_connection_error() {
        let mut server2 = mockito::Server::new_async().await;

        let invalid_url =
            url::Url::parse("http://invalid-host-that-does-not-exist-12345:9999").unwrap();

        let m2 = server2
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(200)
            .with_body("54321")
            .expect(1)
            .create_async()
            .await;

        let client = BitcoinClient::with_urls(
            vec![invalid_url, url::Url::parse(&server2.url()).unwrap()],
            Network::Testnet,
        );

        let result: Result<u64> = client
            .request_with_fallback(
                |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                ReqContext::Get,
                |response| async move { response.json::<u64>().await },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 54321);

        m2.assert_async().await;
    }

    #[tokio::test]
    async fn test_all_urls_fail() {
        let mut server1 = mockito::Server::new_async().await;
        let mut server2 = mockito::Server::new_async().await;

        let m1 = server1
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(503)
            .expect(1)
            .create_async()
            .await;

        let m2 = server2
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let client = BitcoinClient::with_urls(
            vec![
                url::Url::parse(&server1.url()).unwrap(),
                url::Url::parse(&server2.url()).unwrap(),
            ],
            Network::Testnet,
        );

        let result: Result<u64> = client
            .request_with_fallback(
                |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                ReqContext::Get,
                |response| async move { response.json::<u64>().await },
            )
            .await;

        assert!(result.is_err());

        m1.assert_async().await;
        m2.assert_async().await;
    }

    #[tokio::test]
    async fn test_primary_succeeds_no_fallback() {
        let mut server1 = mockito::Server::new_async().await;
        let mut server2 = mockito::Server::new_async().await;

        let m1 = server1
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(200)
            .with_body("99999")
            .expect(1)
            .create_async()
            .await;

        let m2 = server2
            .mock("GET", "/testnet/api/blocks/tip/height")
            .with_status(200)
            .with_body("11111")
            .expect(0)
            .create_async()
            .await;

        let client = BitcoinClient::with_urls(
            vec![
                url::Url::parse(&server1.url()).unwrap(),
                url::Url::parse(&server2.url()).unwrap(),
            ],
            Network::Testnet,
        );

        let result: Result<u64> = client
            .request_with_fallback(
                |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                ReqContext::Get,
                |response| async move { response.json::<u64>().await },
            )
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 99999);

        m1.assert_async().await;
        m2.assert_async().await;
    }

    #[test]
    fn test_fallback_on_rate_limit() {
        let rt = ::tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server1 = mockito::Server::new_async().await;
            let mut server2 = mockito::Server::new_async().await;

            let m1 = server1
                .mock("GET", "/testnet/api/blocks/tip/height")
                .with_status(429)
                .expect(1)
                .create_async()
                .await;

            let m2 = server2
                .mock("GET", "/testnet/api/blocks/tip/height")
                .with_status(200)
                .with_body("88888")
                .expect(1)
                .create_async()
                .await;

            let client = BitcoinClient::with_urls(
                vec![
                    url::Url::parse(&server1.url()).unwrap(),
                    url::Url::parse(&server2.url()).unwrap(),
                ],
                Network::Testnet,
            );

            let result: Result<u64> = client
                .request_with_fallback(
                    |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                    ReqContext::Get,
                    |response| async move { response.json::<u64>().await },
                )
                .await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 88888);

            m1.assert_async().await;
            m2.assert_async().await;
        });
    }

    #[test]
    fn test_fallback_on_request_timeout() {
        let rt = ::tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server1 = mockito::Server::new_async().await;
            let mut server2 = mockito::Server::new_async().await;

            let m1 = server1
                .mock("GET", "/testnet/api/blocks/tip/height")
                .with_status(408)
                .expect(1)
                .create_async()
                .await;

            let m2 = server2
                .mock("GET", "/testnet/api/blocks/tip/height")
                .with_status(200)
                .with_body("77777")
                .expect(1)
                .create_async()
                .await;

            let client = BitcoinClient::with_urls(
                vec![
                    url::Url::parse(&server1.url()).unwrap(),
                    url::Url::parse(&server2.url()).unwrap(),
                ],
                Network::Testnet,
            );

            let result: Result<u64> = client
                .request_with_fallback(
                    |base_url| client.build_api_url(base_url, "/blocks/tip/height"),
                    ReqContext::Get,
                    |response| async move { response.json::<u64>().await },
                )
                .await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 77777);

            m1.assert_async().await;
            m2.assert_async().await;
        });
    }

    #[test]
    fn test_payment_state_from_transactions() {
        init_test_cfg();
        let test_height = 4578915;
        let test_addr = BitcoinAddress::from_str("n4n9CNeCkgtEs8wukKEvWC78eEqK4A3E6d").unwrap();
        let test_amount = 500;
        let mut test_tx = Tx {
            txid: "".into(),
            status: Status {
                block_height: Some(test_height - 7),
                block_time: Some(1731593927),
                block_hash: Some(
                    "000000000061ad7b0d52af77e5a9dbcdc421bf00e93992259f16b2cf2693c4b1".into(),
                ),
                confirmed: true,
            },
            vout: vec![Vout {
                value: 500,
                scriptpubkey_address: Some(test_addr.assume_checked_ref().to_string()),
            }],
        };

        let res_empty =
            payment_state_from_transactions(test_height, vec![], &test_addr, test_amount);
        assert!(matches!(res_empty, Ok(PaymentState::NotFound)));

        let res_paid_confirmed = payment_state_from_transactions(
            test_height,
            vec![test_tx.clone()],
            &test_addr,
            test_amount,
        );
        assert!(matches!(
            res_paid_confirmed,
            Ok(PaymentState::PaidConfirmed(..))
        ));

        test_tx.status.block_height = Some(test_height - 1); // only 2 confirmations
        let res_paid_unconfirmed = payment_state_from_transactions(
            test_height,
            vec![test_tx.clone()],
            &test_addr,
            test_amount,
        );
        assert!(matches!(
            res_paid_unconfirmed,
            Ok(PaymentState::PaidUnconfirmed(..))
        ));

        test_tx.status.block_height = None;
        test_tx.status.block_time = None;
        test_tx.status.block_hash = None;
        let res_paid_confirmed_no_data = payment_state_from_transactions(
            test_height,
            vec![test_tx.clone()],
            &test_addr,
            test_amount,
        );
        assert!(matches!(
            res_paid_confirmed_no_data,
            Err(super::super::Error::ExternalBitcoinApi(Error::InvalidData(
                ..
            )))
        ));

        test_tx.status.confirmed = false;
        let res_in_mem_pool = payment_state_from_transactions(
            test_height,
            vec![test_tx.clone()],
            &test_addr,
            test_amount,
        );
        assert!(matches!(res_in_mem_pool, Ok(PaymentState::InMempool(..))));

        test_tx.vout[0].value = 200;
        let res_not_filled = payment_state_from_transactions(
            test_height,
            vec![test_tx.clone()],
            &test_addr,
            test_amount,
        );
        assert!(matches!(res_not_filled, Ok(PaymentState::NotFound)));
    }

    #[test]
    fn test_check_and_estimate() {
        let rt = ::tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server1 = mockito::Server::new_async().await;
            let utxo_json = json!([{
                "txid": "fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3",
                "vout": 0,
                "value": 1000,
                "status": {
                    "confirmed": true
                }
            }]);

            let m1 = server1
                .mock("GET", "/testnet/api/address/tb1p98hgytlecct3qzfmd9qnf05q03ql032xvpdg9kpwfftej2t95t8s0eyx5k/utxo")
                .with_status(200)
                .with_body(utxo_json.to_string())
                .expect(1)
                .create_async()
                .await;

            let tx_json = json!({
                "txid": "fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3",
                "vout": [{
                    "value": 1000,
                    "scriptpubkey": "512029ee822ff9c61710093b694134be807c41f7c546605a82d82e4a57992965a2cf",
                    "scriptpubkey_address": "tb1p98hgytlecct3qzfmd9qnf05q03ql032xvpdg9kpwfftej2t95t8s0eyx5k",
                }],
            });

            let m2 = server1
                .mock("GET", "/testnet/api/tx/fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3")
                .with_status(200)
                .with_body(tx_json.to_string())
                .expect(1)
                .create_async()
                .await;

            let fee_json = json!({
                "1": 2.0,
                "6": 1.0,
            });

            let m3 = server1
                .mock("GET", "/testnet/api/fee-estimates")
                .with_status(200)
                .with_body(fee_json.to_string())
                .expect(1)
                .create_async()
                .await;

            let client = BitcoinClient::with_urls(
                vec![url::Url::parse(&server1.url()).unwrap()],
                Network::Testnet,
            );

            let estimate = client
                .check_and_estimate_sweep(
                    &BtcDescriptor::new("tr(cPHbchvqgi9ACegotAK34Hr17RokaeEqavMdsRw3XuWtghXBUYU2)#ujfsz6y4").unwrap(),
                    &BitcoinAddress::from_str("tb1qlzxh9zqzc0cfurkwjnua0ar0schh35f3836ngm")
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(estimate.available_funds, 1000);
            assert_eq!(estimate.economy.fee_sat, 99);
            assert_eq!(estimate.fast.fee_sat, 198);

            m1.assert_async().await;
            m2.assert_async().await;
            m3.assert_async().await;
        });
    }

    #[test]
    fn test_sweep_funds() {
        let rt = ::tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server1 = mockito::Server::new_async().await;
            let utxo_json = json!([{
                "txid": "fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3",
                "vout": 0,
                "value": 1000,
                "status": {
                    "confirmed": true
                }
            }]);

            let m1 = server1
                .mock("GET", "/testnet/api/address/tb1p98hgytlecct3qzfmd9qnf05q03ql032xvpdg9kpwfftej2t95t8s0eyx5k/utxo")
                .with_status(200)
                .with_body(utxo_json.to_string())
                .expect(1)
                .create_async()
                .await;

            let tx_json = json!({
                "txid": "fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3",
                "vout": [{
                    "value": 1000,
                    "scriptpubkey": "512029ee822ff9c61710093b694134be807c41f7c546605a82d82e4a57992965a2cf",
                    "scriptpubkey_address": "tb1p98hgytlecct3qzfmd9qnf05q03ql032xvpdg9kpwfftej2t95t8s0eyx5k",
                }],
            });

            let m2 = server1
                .mock("GET", "/testnet/api/tx/fa45e99db4d139383a0d11687ae11c16e9b56633875b6f5d1f12dc80158d45d3")
                .with_status(200)
                .with_body(tx_json.to_string())
                .expect(1)
                .create_async()
                .await;

            let m3 = server1
                .mock("POST", "/testnet/api/tx")
                .with_status(200)
                .with_body("e0f209093038991a03abb19d382f1ee89d21aae87ff1f24e0e71b1c1c0cabd4f")
                .expect(1)
                .create_async()
                .await;

            let client = BitcoinClient::with_urls(
                vec![url::Url::parse(&server1.url()).unwrap()],
                Network::Testnet,
            );

            let res = client
                .sweep_funds(
                    &BtcDescriptor::new("tr(cPHbchvqgi9ACegotAK34Hr17RokaeEqavMdsRw3XuWtghXBUYU2)#ujfsz6y4").unwrap(),
                    &BitcoinAddress::from_str("tb1qlzxh9zqzc0cfurkwjnua0ar0schh35f3836ngm")
                        .unwrap(),
                        50
                )
                .await
                .unwrap();
            assert_eq!(res.tx_id, "e0f209093038991a03abb19d382f1ee89d21aae87ff1f24e0e71b1c1c0cabd4f");
            assert_eq!(res.fee_sat, 50);
            assert_eq!(res.sweep_amount, 950);

            m1.assert_async().await;
            m2.assert_async().await;
            m3.assert_async().await;
        });
    }
}
