use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bcr_common::cashu::{self, State, nut01 as cdk01, nut02 as cdk02};
use bcr_common::client::mint::Client as ExternalMintClient;
use bcr_common::core::NodeId;
use bcr_common::core::signature;
use bcr_common::core::{BillId, keys::to_fee_and_amounts};
use bcr_common::ecash::{self, ProofsMethods};
use bcr_common::wallet;
use bcr_common::wire::borsh::{deserialize_from_str, serialize_as_str};
use bcr_common::wire::quotes::{
    ApplicantActionProjection, EnquireReply, QuoteStatusReply as WireQuoteStatusReply,
    ResolveOffer, SharedBill, StatusReply,
};
use bcr_ebill_core::protocol::{BitcoinAddress, BlockId, Sha256Hash, Sum};
use bcr_ebill_core::{
    application::ServiceTraitBounds, protocol::DateTimeUtc, protocol::SecretKey,
    protocol::blockchain::bill::BillToShareWithExternalParty, protocol::crypto::BcrKeys,
};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::rand::{prelude::SliceRandom, thread_rng};
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::str::FromStr;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

/// Generic result type
pub type Result<T> = std::result::Result<T, super::Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// all errors originating from interacting with the web api
    #[error("External Mint Web API error: {0}")]
    Api(#[from] reqwest::Error),
    /// all errors originating from parsing public keys
    #[error("External Mint Public Key Error")]
    PubKey,
    /// all errors originating from parsing private keys
    #[error("External Mint Private Key Error")]
    PrivateKey,
    /// all errors originating from creating signatures
    #[error("External Mint Signature Error")]
    Signature,
    /// all errors originating from invalid dates
    #[error("External Mint Invalid Date Error")]
    InvalidDate,
    /// all errors originating from invalid mint urls
    #[error("External Mint Invalid Mint Url Error")]
    InvalidMintUrl,
    /// all errors originating from invalid mint request ids
    #[error("External Mint Invalid Mint Request Id Error")]
    InvalidMintRequestId,
    /// all errors originating from invalid keyset ids
    #[error("External Mint Invalid KeySet Id Error")]
    InvalidKeySetId,
    /// all errors originating from invalid tokens
    #[error("External Mint Invalid Token Error")]
    InvalidToken,
    /// all errors originating from tokens and mints not matching
    #[error("External Mint Token and Mint don't match Error")]
    TokenAndMintDontMatch,
    /// all errors originating from cashu amount generation
    #[error("External Mint Amount Error")]
    Amount,
    /// all errors originating from blind message generation
    #[error("External Mint BlindMessage Error")]
    BlindMessage,
    /// an error constructing proofs from minting
    #[error("External Mint ProofConstruction Error")]
    ProofConstruction,
    /// an error minting
    #[error("External Mint Minting Error")]
    Minting,
    /// all errors originating from the quote client
    #[error("External Mint Quote Client Error")]
    QuoteClient,
    /// the AI Credit quote-reissue authority could not be decoded
    #[error("External Mint Quote Reissue Permit Error")]
    InvalidQuoteReissuePermit,
    /// all errors originating from the key client
    #[error("External Mint Key Client Error")]
    KeyClient,
    /// all errors originating from the swap client
    #[error("External Mint Swap Client Error")]
    SwapClient,
    /// all errors originating from the clowder client
    #[error("External Mint Clowder Client Error")]
    ClowderClient,
    /// all errors originating from an invalid payment address in a mint request to pay
    #[error("External Mint Request To Pay Payment Address Error")]
    InvalidMintRequestToPayPaymentAddress,
    /// all errors originating from an alpha returning no betas
    #[error("External Mint No Betas Error")]
    NoBetas,
}

#[cfg(test)]
use mockall::automock;

use crate::get_config;

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait MintClientApi: ServiceTraitBounds {
    /// Check if the given proofs were already spent
    async fn check_if_proofs_are_spent(
        &self,
        mint_url: &url::Url,
        proofs: &str,
        keyset_id: &str,
    ) -> Result<bool>;
    /// Mint and return encoded token
    async fn mint(
        &self,
        bill_id: &BillId,
        mint_url: &url::Url,
        keyset: ecash::KeySet,
        quote_id: &Uuid,
        private_key: &SecretKey,
        blinded_messages: Vec<cashu::BlindedMessage>,
        secrets: Vec<cashu::secret::Secret>,
        rs: Vec<cashu::SecretKey>,
    ) -> Result<String>;
    /// Check keyset info for a given keyset id with a given mint
    async fn get_keyset_info(&self, mint_url: &url::Url, keyset_id: &str) -> Result<ecash::KeySet>;
    /// Request to mint a bill with a given mint
    async fn enquire_mint_quote(
        &self,
        mint_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        requester_keys: &BcrKeys,
    ) -> Result<Uuid>;
    /// Request a replacement for a terminal quote with independently signed AI Credit authority.
    async fn enquire_mint_quote_reissue(
        &self,
        mint_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        requester_keys: &BcrKeys,
        signed_reissue_permit_json: &str,
    ) -> Result<Uuid>;
    /// Look up a quote for a mint
    async fn lookup_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
    ) -> Result<MintQuoteLookupReply>;
    /// Resolve quote from mint
    async fn resolve_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
        resolve: ResolveMintOffer,
    ) -> Result<()>;
    /// Cancel request to mint
    async fn cancel_quote_for_mint(&self, mint_url: &url::Url, quote_id: &Uuid) -> Result<()>;
    /// Validate the given btc address from a mint request to pay against a random beta of our alpha mint
    async fn validate_payment_address_from_mint(
        &self,
        mint_url: &url::Url,
        address_to_validate: &BitcoinAddress,
        bill_id: &BillId,
        block_id: BlockId,
        previous_block_hash: &Sha256Hash,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct MintClient {}

const REISSUE_ENQUIRE_SCHEMA_VERSION: &str = "credit-quote-reissue-enquiry-v1";
const REISSUE_ENQUIRE_ACTION: &str = "reissue_denied_quote_after_reviewed_correction";
const REISSUE_PERMIT_SCHEMA_VERSION: &str = "credit-quote-reissue-permit-v1";
const REISSUE_PERMIT_SIGNATURE_ALGORITHM: &str = "Ed25519";
const REISSUE_ENQUIRE_PATH: &str = "/v1/quote/ebill/reissue";
const MAX_REISSUE_PERMIT_JSON_BYTES: usize = 32 * 1024;
const MAX_REISSUE_REPLY_BYTES: usize = 16 * 1024;
const REISSUE_REQUEST_TIMEOUT_SECONDS: u64 = 15;
const MAX_REISSUE_PERMIT_TTL: Duration = Duration::days(1);
const REISSUE_PERMIT_CLOCK_SKEW: Duration = Duration::seconds(30);

fn deserialize_canonical_uuid<'de, D>(deserializer: D) -> std::result::Result<Uuid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let uuid = Uuid::parse_str(&value).map_err(serde::de::Error::custom)?;
    let bytes = value.as_bytes();
    if uuid.to_string() != value
        || !matches!(bytes.get(14), Some(b'1'..=b'8'))
        || !matches!(bytes.get(19), Some(b'8' | b'9' | b'a' | b'b'))
    {
        return Err(serde::de::Error::custom(
            "UUID must use canonical lowercase form",
        ));
    }
    Ok(uuid)
}

// These versioned wire types mirror bcr-common's next quote contract. Core intentionally keeps
// its released bcr-common pin until that coordinated public revision exists; the fixture below
// locks the Borsh bytes meanwhile.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreditQuoteReissuePermit {
    schema_version: String,
    key_id: String,
    mint_id: String,
    #[borsh(
        serialize_with = "serialize_as_str",
        deserialize_with = "deserialize_from_str"
    )]
    #[serde(deserialize_with = "deserialize_canonical_uuid")]
    previous_mint_quote_id: Uuid,
    #[borsh(
        serialize_with = "serialize_as_str",
        deserialize_with = "deserialize_from_str"
    )]
    #[serde(deserialize_with = "deserialize_canonical_uuid")]
    reissued_mint_quote_id: Uuid,
    credit_program_version: String,
    credit_program_digest: String,
    case_id: String,
    bill_id: String,
    bill_state_digest: String,
    holder_ref: String,
    #[borsh(
        serialize_with = "serialize_as_str",
        deserialize_with = "deserialize_from_str"
    )]
    #[serde(deserialize_with = "deserialize_canonical_uuid")]
    review_request_id: Uuid,
    contested_decision_result_digest: String,
    corrected_submission_digest: String,
    issued_at: String,
    expires_at: String,
    #[borsh(
        serialize_with = "serialize_as_str",
        deserialize_with = "deserialize_from_str"
    )]
    #[serde(deserialize_with = "deserialize_canonical_uuid")]
    nonce: Uuid,
    action: String,
    synthetic: bool,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignedCreditQuoteReissuePermit {
    permit: CreditQuoteReissuePermit,
    permit_digest: String,
    signature_algorithm: String,
    signature: String,
}

#[derive(Debug, borsh::BorshSerialize, borsh::BorshDeserialize)]
struct ReissueEnquireRequestV1 {
    schema_version: String,
    action: String,
    content: SharedBill,
    #[borsh(
        serialize_with = "serialize_as_str",
        deserialize_with = "deserialize_from_str"
    )]
    minting_pubkey: cashu::PublicKey,
    signed_permit: SignedCreditQuoteReissuePermit,
}

#[derive(Debug, Serialize)]
struct SignedReissueEnquireRequestV1 {
    content: String,
    signature: bitcoin::secp256k1::schnorr::Signature,
}

#[cfg(not(target_arch = "wasm32"))]
async fn post_quote_reissue(
    url: url::Url,
    signed: &SignedReissueEnquireRequestV1,
) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(
            REISSUE_REQUEST_TIMEOUT_SECONDS,
        ))
        .build()
        .map_err(|_| Error::QuoteClient)?;
    let mut response = client
        .post(url.clone())
        .json(signed)
        .send()
        .await
        .map_err(|error| {
            log::error!("Error enquiring to reissue mint quote at {url}: {error}");
            Error::QuoteClient
        })?;
    if !response.status().is_success() {
        let status = response.status();
        log::error!("Mint refused quote reissue at {url}: {status}");
        return Err(Error::QuoteClient.into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        log::error!("Mint interrupted its quote reissue response at {url}: {error}");
        Error::QuoteClient
    })? {
        if body.len() + chunk.len() > MAX_REISSUE_REPLY_BYTES {
            log::error!("Mint quote reissue response exceeded its size bound at {url}");
            return Err(Error::QuoteClient.into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(target_arch = "wasm32")]
async fn post_quote_reissue(
    url: url::Url,
    signed: &SignedReissueEnquireRequestV1,
) -> Result<Vec<u8>> {
    use js_sys::{Reflect, Uint8Array};
    use tokio_with_wasm::alias as tokio;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        AbortController, ReadableStreamDefaultReader, Request, RequestInit, RequestRedirect,
        Response,
    };

    let request_body = serde_json::to_string(signed).map_err(|_| Error::QuoteClient)?;
    let controller = AbortController::new().map_err(|_| Error::QuoteClient)?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&request_body));
    init.set_redirect(RequestRedirect::Error);
    init.set_signal(Some(&controller.signal()));
    let request =
        Request::new_with_str_and_init(url.as_str(), &init).map_err(|_| Error::QuoteClient)?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|_| Error::QuoteClient)?;

    let fetch = async {
        let window = web_sys::window().ok_or(Error::QuoteClient)?;
        let response = JsFuture::from(window.fetch_with_request(&request))
            .await
            .map_err(|_| Error::QuoteClient)?;
        let response: Response = response.dyn_into().map_err(|_| Error::QuoteClient)?;
        if !response.ok() {
            log::error!("Mint refused quote reissue at {url}: {}", response.status());
            return Err(Error::QuoteClient.into());
        }

        let Some(stream) = response.body() else {
            return Ok(Vec::new());
        };
        let reader: ReadableStreamDefaultReader = stream
            .get_reader()
            .dyn_into()
            .map_err(|_| Error::QuoteClient)?;
        let mut body = Vec::new();
        loop {
            let result = JsFuture::from(reader.read())
                .await
                .map_err(|_| Error::QuoteClient)?;
            let done = Reflect::get(&result, &JsValue::from_str("done"))
                .map_err(|_| Error::QuoteClient)?
                .as_bool()
                .ok_or(Error::QuoteClient)?;
            if done {
                reader.release_lock();
                return Ok(body);
            }
            let value = Reflect::get(&result, &JsValue::from_str("value"))
                .map_err(|_| Error::QuoteClient)?;
            let chunk = Uint8Array::new(&value);
            let chunk_len = chunk.length() as usize;
            if chunk_len > MAX_REISSUE_REPLY_BYTES.saturating_sub(body.len()) {
                let _ = reader.cancel();
                log::error!("Mint quote reissue response exceeded its size bound at {url}");
                return Err(Error::QuoteClient.into());
            }
            let start = body.len();
            body.resize(start + chunk_len, 0);
            chunk.copy_to(&mut body[start..]);
        }
    };

    match tokio::time::timeout(
        std::time::Duration::from_secs(REISSUE_REQUEST_TIMEOUT_SECONDS),
        fetch,
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            controller.abort();
            log::error!("Mint quote reissue request timed out at {url}");
            Err(Error::QuoteClient.into())
        }
    }
}

fn is_canonical_text(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= 200 && value.nfc().collect::<String>() == value
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_quote_reissue_permit(
    value: &str,
    expected_bill_id: &BillId,
    expected_holder_ref: Option<&NodeId>,
) -> Result<SignedCreditQuoteReissuePermit> {
    if value.len() > MAX_REISSUE_PERMIT_JSON_BYTES {
        return Err(Error::InvalidQuoteReissuePermit.into());
    }
    let signed: SignedCreditQuoteReissuePermit =
        serde_json::from_str(value).map_err(|_| Error::InvalidQuoteReissuePermit)?;
    let permit = &signed.permit;
    let issued_at = chrono::DateTime::parse_from_rfc3339(&permit.issued_at)
        .map_err(|_| Error::InvalidQuoteReissuePermit)?;
    let expires_at = chrono::DateTime::parse_from_rfc3339(&permit.expires_at)
        .map_err(|_| Error::InvalidQuoteReissuePermit)?;
    let now = Utc::now();
    let text_fields = [
        permit.key_id.as_str(),
        permit.mint_id.as_str(),
        permit.credit_program_version.as_str(),
        permit.case_id.as_str(),
        permit.bill_id.as_str(),
        permit.holder_ref.as_str(),
    ];
    let digest_fields = [
        permit.credit_program_digest.as_str(),
        permit.bill_state_digest.as_str(),
        permit.contested_decision_result_digest.as_str(),
        permit.corrected_submission_digest.as_str(),
        signed.permit_digest.as_str(),
    ];
    let signature_is_valid_base64 = signed.signature.len() <= 128
        && STANDARD
            .decode(&signed.signature)
            .is_ok_and(|signature| signature.len() == 64);
    if permit.schema_version != REISSUE_PERMIT_SCHEMA_VERSION
        || permit.action != REISSUE_ENQUIRE_ACTION
        || !permit.synthetic
        || signed.signature_algorithm != REISSUE_PERMIT_SIGNATURE_ALGORITHM
        || text_fields.iter().any(|field| !is_canonical_text(field))
        || digest_fields.iter().any(|digest| !is_sha256_digest(digest))
        || permit.bill_id != expected_bill_id.to_string()
        || expected_holder_ref.is_some_and(|holder| permit.holder_ref != holder.to_string())
        || permit.previous_mint_quote_id == permit.reissued_mint_quote_id
        || issued_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true) != permit.issued_at
        || expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true) != permit.expires_at
        || expires_at <= issued_at
        || expires_at - issued_at > MAX_REISSUE_PERMIT_TTL
        || issued_at > now + REISSUE_PERMIT_CLOCK_SKEW
        || !signature_is_valid_base64
    {
        return Err(Error::InvalidQuoteReissuePermit.into());
    }
    Ok(signed)
}

pub(crate) fn validate_quote_reissue_permit(
    value: &str,
    expected_bill_id: &BillId,
    expected_holder_ref: &NodeId,
) -> Result<Uuid> {
    parse_quote_reissue_permit(value, expected_bill_id, Some(expected_holder_ref))
        .map(|signed| signed.permit.reissued_mint_quote_id)
}

impl ServiceTraitBounds for MintClient {}

#[cfg(test)]
impl ServiceTraitBounds for MockMintClientApi {}

impl MintClient {
    pub fn new() -> Self {
        Self {}
    }

    pub fn client(&self, mint_url: &url::Url) -> Result<ExternalMintClient> {
        let mint_client = ExternalMintClient::new(mint_url.to_owned());
        Ok(mint_client)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl MintClientApi for MintClient {
    async fn check_if_proofs_are_spent(
        &self,
        mint_url: &url::Url,
        proofs: &str,
        keyset_id: &str,
    ) -> Result<bool> {
        let token_mint_url =
            cashu::MintUrl::from_str(mint_url.as_str()).map_err(|_| Error::InvalidMintUrl)?;
        let token = wallet::Token::from_str(proofs).map_err(|_| Error::InvalidToken)?;

        if Some(token_mint_url) != token.mint_url() {
            return Err(Error::InvalidToken.into());
        }

        let keyset_id_parsed = cdk02::Id::from_str(keyset_id).map_err(|e| {
            log::error!("Error parsing keyset id {keyset_id} for {mint_url}: {e}");
            Error::InvalidKeySetId
        })?;

        let keyset_info = self
            .client(mint_url)?
            .keyset_info(keyset_id_parsed)
            .await
            .map_err(|e| {
                log::error!("Error getting keyset info from {mint_url}: {e}");
                Error::KeyClient
            })?;

        let ys = token
            .proofs(&[keyset_info])
            .map_err(|_| Error::InvalidToken)?
            .ys()
            .map_err(|_| Error::PubKey)?;

        let proof_states = self.client(mint_url)?.check_state(ys).await.map_err(|e| {
            log::error!("Error checking if proofs are spent at {mint_url}: {e}");
            Error::SwapClient
        })?;
        // all proofs have to be spent
        let proofs_spent = proof_states
            .iter()
            .all(|ps| matches!(ps.state, State::Spent));
        Ok(proofs_spent)
    }

    async fn mint(
        &self,
        bill_id: &BillId,
        mint_url: &url::Url,
        keyset: ecash::KeySet,
        quote_id: &Uuid,
        private_key: &SecretKey,
        blinded_messages: Vec<cashu::BlindedMessage>,
        secrets: Vec<cashu::secret::Secret>,
        rs: Vec<cashu::SecretKey>,
    ) -> Result<String> {
        let token_mint_url =
            cashu::MintUrl::from_str(mint_url.as_str()).map_err(|_| Error::InvalidMintUrl)?;
        let secret_key = cdk01::SecretKey::from_hex(private_key.display_secret().to_string())
            .map_err(|_| Error::PrivateKey)?;
        let qid = quote_id.to_owned();
        let clowder_id = self
            .client(mint_url)?
            .get_info()
            .await
            .map_err(|e| {
                log::error!("Error getting mint clowder info on mint {mint_url}: {e}");
                Error::ClowderClient
            })?
            .node_id;
        let clowder_node_id = NodeId::new(
            clowder_id.deref().to_owned(),
            get_config().bitcoin_network(),
        );
        let currency = self
            .client(mint_url)?
            .keyset_info(keyset.id)
            .await
            .map_err(|e| {
                log::error!("Error getting keyset info from {mint_url}: {e}");
                Error::Minting
            })?
            .unit;

        // mint
        let blinded_signatures = self
            .client(mint_url)?
            .ebill_mint(qid, blinded_messages, secret_key)
            .await
            .map_err(|e| {
                log::error!("Error minting at mint {mint_url}: {e}");
                Error::Minting
            })?;

        // create proofs
        let proofs = cashu::dhke::construct_proofs(blinded_signatures, rs, secrets, &keyset.keys)
            .map_err(|e| {
            log::error!("Couldn't construct proofs for {quote_id}: {e}");
            Error::ProofConstruction
        })?;

        // generate token from proofs
        let token = wallet::Token::BitcrV5(
            wallet::BitcrTokenV5::new(
                clowder_node_id,
                currency,
                proofs.into_iter().map(|p| p.into()).collect(),
            )
            .with_memo(bill_id.to_string())
            .with_mint_url(token_mint_url.to_string()),
        );

        Ok(token.to_string())
    }

    async fn get_keyset_info(&self, mint_url: &url::Url, keyset_id: &str) -> Result<ecash::KeySet> {
        let keyset_id_parsed = cdk02::Id::from_str(keyset_id).map_err(|e| {
            log::error!("Error parsing keyset id {keyset_id} for {mint_url}: {e}");
            Error::InvalidKeySetId
        })?;
        #[allow(deprecated)]
        let keyset = self
            .client(mint_url)?
            .keys_v1(keyset_id_parsed)
            .await
            .map_err(|e| {
                log::error!("Error getting keyset info at mint {mint_url}: {e}");
                Error::KeyClient
            })?;
        Ok(keyset)
    }

    async fn enquire_mint_quote(
        &self,
        mint_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        requester_keys: &BcrKeys,
    ) -> Result<Uuid> {
        let shared_bill = map_shared_bill(bill_to_share);

        let public_key = cdk01::PublicKey::from_hex(requester_keys.get_public_key())
            .map_err(|_| Error::PubKey)?;

        let mint_request_id = self
            .client(mint_url)?
            .enquire(shared_bill, public_key, &requester_keys.get_key_pair())
            .await
            .map_err(|e| {
                log::error!("Error enquiring to mint {mint_url}: {e}");
                Error::QuoteClient
            })?;
        Ok(mint_request_id)
    }

    async fn enquire_mint_quote_reissue(
        &self,
        mint_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        requester_keys: &BcrKeys,
        signed_reissue_permit_json: &str,
    ) -> Result<Uuid> {
        let signed_permit =
            parse_quote_reissue_permit(signed_reissue_permit_json, &bill_to_share.bill_id, None)?;
        let minting_pubkey = cdk01::PublicKey::from_hex(requester_keys.get_public_key())
            .map_err(|_| Error::PubKey)?;
        let request = ReissueEnquireRequestV1 {
            schema_version: REISSUE_ENQUIRE_SCHEMA_VERSION.to_owned(),
            action: REISSUE_ENQUIRE_ACTION.to_owned(),
            content: map_shared_bill(bill_to_share),
            minting_pubkey,
            signed_permit,
        };
        let (content, request_signature) =
            signature::serialize_n_schnorr_sign_borsh_msg(&request, &requester_keys.get_key_pair())
                .map_err(|_| Error::Signature)?;
        let signed = SignedReissueEnquireRequestV1 {
            content,
            signature: request_signature,
        };
        let url = mint_url
            .join(REISSUE_ENQUIRE_PATH)
            .map_err(|_| Error::InvalidMintUrl)?;
        let body = post_quote_reissue(url, &signed).await?;
        let response = serde_json::from_slice::<EnquireReply>(&body).map_err(|error| {
            log::error!("Mint returned an invalid quote reissue at {mint_url}: {error}");
            Error::QuoteClient
        })?;
        Ok(response.id)
    }

    async fn lookup_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
    ) -> Result<MintQuoteLookupReply> {
        let reply = self
            .client(mint_url)?
            .lookup(quote_id.to_owned())
            .await
            .map_err(|e| {
                log::error!("Error looking up request on mint {mint_url}: {e}");
                Error::QuoteClient
            })?;
        MintQuoteLookupReply::try_from(reply)
    }

    async fn resolve_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
        resolve: ResolveMintOffer,
    ) -> Result<()> {
        match resolve {
            ResolveMintOffer::Accept => {
                self.client(mint_url)?
                    .accept_offer(quote_id.to_owned())
                    .await
                    .map_err(|e| {
                        log::error!("Error accepting request on mint {mint_url}: {e}");
                        Error::QuoteClient
                    })?;
            }
            ResolveMintOffer::Reject => {
                self.client(mint_url)?
                    .reject_offer(quote_id.to_owned())
                    .await
                    .map_err(|e| {
                        log::error!("Error rejecting request on mint {mint_url}: {e}");
                        Error::QuoteClient
                    })?;
            }
        };
        Ok(())
    }

    async fn cancel_quote_for_mint(&self, mint_url: &url::Url, quote_id: &Uuid) -> Result<()> {
        self.client(mint_url)?
            .cancel_enquiry(quote_id.to_owned())
            .await
            .map_err(|e| {
                log::error!("Error cancelling request on mint {mint_url}: {e}");
                Error::QuoteClient
            })?;
        Ok(())
    }

    async fn validate_payment_address_from_mint(
        &self,
        mint_url: &url::Url,
        address_to_validate: &BitcoinAddress,
        bill_id: &BillId,
        block_id: BlockId,
        previous_block_hash: &Sha256Hash,
    ) -> Result<()> {
        let mint_info = self.client(mint_url)?.get_info().await.map_err(|e| {
            log::error!("Error getting mint clowder info on mint {mint_url}: {e}");
            Error::ClowderClient
        })?;
        let betas = self.client(mint_url)?.get_betas().await.map_err(|e| {
            log::error!("Error getting betas on mint {mint_url}: {e}");
            Error::ClowderClient
        })?;
        let Some(random_beta) = betas.mints.choose(&mut thread_rng()) else {
            return Err(Error::NoBetas.into());
        };

        let beta_url = &random_beta.mint;
        let derived_payment_address_from_beta = self
            .client(beta_url)?
            .derive_ebill_payment_address(
                *mint_info.node_id,
                bill_id.to_owned(),
                block_id.inner(),
                sha256::Hash::from_byte_array(previous_block_hash.decode_to_array()),
            )
            .await
            .map_err(|e| {
                log::error!("Error deriving payment address on mint {beta_url}: {e}");
                Error::ClowderClient
            })?;

        if address_to_validate != &derived_payment_address_from_beta.payment_address {
            log::error!(
                "Error deriving payment address on mint {beta_url}: Addresses don't match: to_validate: {}, derived: {}",
                address_to_validate.assume_checked_ref(),
                derived_payment_address_from_beta
                    .payment_address
                    .assume_checked_ref()
            );
            return Err(Error::InvalidMintRequestToPayPaymentAddress.into());
        }

        Ok(())
    }
}

pub fn generate_blinds(
    keyset: &ecash::KeySet,
    discounted_amount: Sum,
) -> Result<(
    Vec<cashu::BlindedMessage>,
    Vec<cashu::secret::Secret>,
    Vec<cashu::SecretKey>,
)> {
    let amount = cashu::Amount::from(discounted_amount.as_sat());
    let amounts: Vec<cashu::Amount> = amount
        .split(&to_fee_and_amounts(keyset))
        .map_err(|_| Error::Amount)?;
    let mut blinded_messages = Vec::with_capacity(amounts.len());
    let mut secrets = Vec::with_capacity(amounts.len());
    let mut rs = Vec::with_capacity(amounts.len());

    for amount in amounts {
        let blind = generate_blind(keyset.id, amount)?;
        blinded_messages.push(blind.0);
        secrets.push(blind.1);
        rs.push(blind.2);
    }

    Ok((blinded_messages, secrets, rs))
}

pub fn generate_blind(
    kid: cashu::Id,
    amount: cashu::Amount,
) -> Result<(
    cashu::BlindedMessage,
    cashu::secret::Secret,
    cashu::SecretKey,
)> {
    let secret = cashu::secret::Secret::new(hex::encode(rand::random::<[u8; 32]>()));
    let (b_, r) =
        cashu::dhke::blind_message(secret.as_bytes(), None).map_err(|_| Error::BlindMessage)?;
    Ok((cashu::BlindedMessage::new(amount, kid, b_), secret, r))
}

#[derive(Debug, Clone)]
pub enum ResolveMintOffer {
    Accept,
    Reject,
}

impl From<ResolveMintOffer> for ResolveOffer {
    fn from(value: ResolveMintOffer) -> Self {
        match value {
            ResolveMintOffer::Accept => ResolveOffer::Accept,
            ResolveMintOffer::Reject => ResolveOffer::Reject,
        }
    }
}

#[derive(Debug, Clone)]
pub enum QuoteStatusReply {
    Pending,
    Denied {
        tstamp: DateTimeUtc,
    },
    Offered {
        keyset_id: cdk02::Id,
        expiration_date: DateTimeUtc,
        discounted: bitcoin::Amount,
    },
    Accepted {
        keyset_id: cdk02::Id,
    },
    MintingEnabled {
        keyset_id: cdk02::Id,
        minted_amount: cashu::Amount,
    },
    Rejected {
        tstamp: DateTimeUtc,
    },
    Cancelled {
        tstamp: DateTimeUtc,
    },
    Expired {
        tstamp: DateTimeUtc,
    },
    FailedEbillValidation {
        keyset_id: cdk02::Id,
    },
}

#[derive(Debug, Clone)]
pub struct MintQuoteLookupReply {
    pub quote: QuoteStatusReply,
    pub applicant_action: Option<ApplicantActionProjection>,
}

impl From<QuoteStatusReply> for MintQuoteLookupReply {
    fn from(quote: QuoteStatusReply) -> Self {
        Self {
            quote,
            applicant_action: None,
        }
    }
}

impl TryFrom<WireQuoteStatusReply> for MintQuoteLookupReply {
    type Error = super::Error;

    fn try_from(value: WireQuoteStatusReply) -> std::result::Result<Self, Self::Error> {
        if value
            .applicant_action
            .as_ref()
            .is_some_and(|action| !is_sha256_digest(&action.revision_digest))
        {
            return Err(Error::QuoteClient.into());
        }
        Ok(Self {
            quote: value.quote.into(),
            applicant_action: value.applicant_action,
        })
    }
}

impl From<StatusReply> for QuoteStatusReply {
    fn from(value: StatusReply) -> Self {
        match value {
            StatusReply::Pending => QuoteStatusReply::Pending,
            StatusReply::Denied { tstamp } => QuoteStatusReply::Denied { tstamp },
            StatusReply::Offered {
                keyset_id,
                expiration_date,
                discounted,
                ..
            } => QuoteStatusReply::Offered {
                keyset_id,
                expiration_date,
                discounted,
            },
            StatusReply::Accepted { keyset_id, .. } => QuoteStatusReply::Accepted { keyset_id },
            StatusReply::Rejected { tstamp, .. } => QuoteStatusReply::Rejected { tstamp },
            StatusReply::Canceled { tstamp } => QuoteStatusReply::Cancelled { tstamp },
            StatusReply::OfferExpired { tstamp, .. } => QuoteStatusReply::Expired { tstamp },
            StatusReply::FailedEbillValidation { keyset_id, .. } => {
                QuoteStatusReply::FailedEbillValidation { keyset_id }
            }
            StatusReply::MintingEnabled {
                keyset_id,
                minted_amount,
                ..
            } => QuoteStatusReply::MintingEnabled {
                keyset_id,
                minted_amount,
            },
        }
    }
}

fn map_shared_bill(
    bill_to_share: BillToShareWithExternalParty,
) -> bcr_common::wire::quotes::SharedBill {
    bcr_common::wire::quotes::SharedBill {
        bill_id: bill_to_share.bill_id,
        data: bill_to_share.data,
        file_urls: bill_to_share.file_urls,
        hash: bill_to_share.hash.to_string(),
        signature: bill_to_share.signature.to_string(),
        receiver: bill_to_share.receiver.into(),
    }
}

#[cfg(test)]
mod quote_reissue_tests {
    use std::str::FromStr;

    use bitcoin::hashes::{Hash as _, sha256};

    use super::*;

    fn request_fixture() -> ReissueEnquireRequestV1 {
        ReissueEnquireRequestV1 {
            schema_version: REISSUE_ENQUIRE_SCHEMA_VERSION.to_owned(),
            action: REISSUE_ENQUIRE_ACTION.to_owned(),
            content: SharedBill {
                bill_id: BillId::from_str("bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY")
                    .unwrap(),
                data: "encrypted-bill".to_owned(),
                file_urls: vec![url::Url::parse("https://example.test/evidence.pdf").unwrap()],
                hash: "bill-hash".to_owned(),
                signature: "bill-signature".to_owned(),
                receiver: bitcoin::PublicKey::from_str(
                    "026423b7d36d05b8d50a89a1b4ef2a06c88bcd2c5e650f25e122fa682d3b39686c",
                )
                .unwrap(),
            },
            minting_pubkey: cashu::PublicKey::from_str(
                "026423b7d36d05b8d50a89a1b4ef2a06c88bcd2c5e650f25e122fa682d3b39686c",
            )
            .unwrap(),
            signed_permit: SignedCreditQuoteReissuePermit {
                permit: CreditQuoteReissuePermit {
                    schema_version: "credit-quote-reissue-permit-v1".to_owned(),
                    key_id: "local-testnet-key".to_owned(),
                    mint_id: "local-wildcat".to_owned(),
                    previous_mint_quote_id: Uuid::parse_str("11111111-1111-4111-8111-111111111111")
                        .unwrap(),
                    reissued_mint_quote_id: Uuid::parse_str("22222222-2222-4222-8222-222222222222")
                        .unwrap(),
                    credit_program_version: "coffee-v1".to_owned(),
                    credit_program_digest: format!("sha256:{}", "1".repeat(64)),
                    case_id: "case-1".to_owned(),
                    bill_id: "bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY".to_owned(),
                    bill_state_digest: format!("sha256:{}", "2".repeat(64)),
                    holder_ref: "bitcrt-holder".to_owned(),
                    review_request_id: Uuid::parse_str("33333333-3333-4333-8333-333333333333")
                        .unwrap(),
                    contested_decision_result_digest: format!("sha256:{}", "3".repeat(64)),
                    corrected_submission_digest: format!("sha256:{}", "4".repeat(64)),
                    issued_at: "2026-08-24T00:00:00.000Z".to_owned(),
                    expires_at: "2026-08-25T00:00:00.000Z".to_owned(),
                    nonce: Uuid::parse_str("44444444-4444-4444-8444-444444444444")
                        .unwrap(),
                    action: REISSUE_ENQUIRE_ACTION.to_owned(),
                    synthetic: true,
                },
                permit_digest: format!("sha256:{}", "5".repeat(64)),
                signature_algorithm: "Ed25519".to_owned(),
                signature: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
                    .to_owned(),
            },
        }
    }

    fn signed_request_fixture() -> SignedReissueEnquireRequestV1 {
        let keys = BcrKeys::new();
        let (content, signature) =
            signature::serialize_n_schnorr_sign_borsh_msg(&request_fixture(), &keys.get_key_pair())
                .unwrap();
        SignedReissueEnquireRequestV1 { content, signature }
    }

    #[test]
    fn quote_reissue_borsh_matches_mint_contract_fixture() {
        let bytes = borsh::to_vec(&request_fixture()).unwrap();
        assert_eq!(
            sha256::Hash::hash(&bytes).to_string(),
            "a406744b2d1762df6e6b02f566a9dad2da9bb2fcae5925f9980fad68f6c1a91d"
        );
    }

    #[test]
    fn quote_reissue_permit_json_is_strictly_bounded() {
        let request = request_fixture();
        let json = serde_json::to_string(&request.signed_permit).unwrap();
        parse_quote_reissue_permit(&json, &request.content.bill_id, None).unwrap();

        let mut unknown: serde_json::Value = serde_json::from_str(&json).unwrap();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        assert!(
            parse_quote_reissue_permit(&unknown.to_string(), &request.content.bill_id, None)
                .is_err()
        );
        assert!(
            parse_quote_reissue_permit(
                &"x".repeat(MAX_REISSUE_PERMIT_JSON_BYTES + 1),
                &request.content.bill_id,
                None,
            )
            .is_err()
        );

        for (path, invalid_value) in [
            (
                &["permit", "billId"][..],
                serde_json::Value::String("another-bill".to_owned()),
            ),
            (
                &["permit", "nonce"][..],
                serde_json::Value::String("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".to_owned()),
            ),
            (
                &["permit", "keyId"][..],
                serde_json::Value::String("e\u{301}".to_owned()),
            ),
            (
                &["permit", "expiresAt"][..],
                serde_json::Value::String("2026-08-25T00:00:00.001Z".to_owned()),
            ),
            (
                &["permit", "issuedAt"][..],
                serde_json::Value::String("2026-08-24T00:00:00Z".to_owned()),
            ),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_str(&json).unwrap();
            let mut cursor = &mut invalid;
            for segment in &path[..path.len() - 1] {
                cursor = cursor.get_mut(*segment).unwrap();
            }
            cursor[path[path.len() - 1]] = invalid_value;
            assert!(
                parse_quote_reissue_permit(&invalid.to_string(), &request.content.bill_id, None)
                    .is_err()
            );
        }

        let mut future: serde_json::Value = serde_json::from_str(&json).unwrap();
        let issued_at = Utc::now() + Duration::minutes(5);
        let expires_at = issued_at + Duration::minutes(15);
        future["permit"]["issuedAt"] = serde_json::Value::String(
            issued_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
        future["permit"]["expiresAt"] = serde_json::Value::String(
            expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
        assert!(
            parse_quote_reissue_permit(&future.to_string(), &request.content.bill_id, None)
                .is_err()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn quote_reissue_http_does_not_forward_signed_body_on_redirect() {
        let mut server = mockito::Server::new_async().await;
        let redirected = server
            .mock("POST", "/redirect-target")
            .expect(0)
            .create_async()
            .await;
        let redirect = server
            .mock("POST", REISSUE_ENQUIRE_PATH)
            .with_status(307)
            .with_header("location", &format!("{}/redirect-target", server.url()))
            .expect(1)
            .create_async()
            .await;
        let url = url::Url::parse(&format!("{}{}", server.url(), REISSUE_ENQUIRE_PATH)).unwrap();

        assert!(
            post_quote_reissue(url, &signed_request_fixture())
                .await
                .is_err()
        );
        redirect.assert_async().await;
        redirected.assert_async().await;
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn quote_reissue_http_rejects_oversized_response() {
        let mut server = mockito::Server::new_async().await;
        let response = server
            .mock("POST", REISSUE_ENQUIRE_PATH)
            .with_status(200)
            .with_body("x".repeat(MAX_REISSUE_REPLY_BYTES + 1))
            .expect(1)
            .create_async()
            .await;
        let url = url::Url::parse(&format!("{}{}", server.url(), REISSUE_ENQUIRE_PATH)).unwrap();

        assert!(
            post_quote_reissue(url, &signed_request_fixture())
                .await
                .is_err()
        );
        response.assert_async().await;
    }
}
