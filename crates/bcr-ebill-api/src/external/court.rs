use async_trait::async_trait;
use bcr_ebill_core::{
    application::ServiceTraitBounds, protocol::blockchain::bill::BillToShareWithExternalParty,
    protocol::crypto::BcrKeys,
};
use bitcoin::hashes::{Hash, sha256::Hash as Sha256};
use bitcoin::secp256k1::{Message, PublicKey, SECP256K1};
use borsh::to_vec;
use borsh_derive::BorshSerialize;
use serde::Serialize;
use thiserror::Error;

/// Generic result type
pub type Result<T> = std::result::Result<T, super::Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// all errors originating from interacting with the court web api
    #[error("External Court API error: {0}")]
    Api(#[from] reqwest::Error),
    /// all signature errors
    #[error("External Court Signature Error: {0}")]
    Signature(#[from] bitcoin::secp256k1::Error),
    /// all borsh errors
    #[error("External Court Borsh Error")]
    Borsh(#[from] borsh::io::Error),
}

#[cfg(test)]
use mockall::automock;

#[derive(Debug, Serialize, BorshSerialize)]
pub struct ReceiveBillRequest {
    pub content: BillToShareWithExternalParty,
    #[borsh(
        serialize_with = "bcr_ebill_core::protocol::serialization::serialize_pubkey",
        deserialize_with = "bcr_ebill_core::protocol::serialization::deserialize_pubkey"
    )]
    pub public_key: PublicKey,
}

#[derive(Debug, Serialize)]
pub struct SignedReceiveBillRequest {
    pub request: ReceiveBillRequest,
    pub signature: bitcoin::secp256k1::schnorr::Signature,
}

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait CourtClientApi: ServiceTraitBounds {
    /// Create request, sign it and send it to the given court URL endpoint
    async fn share_with_court(
        &self,
        court_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        sharer_keys: &BcrKeys,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Default)]
pub struct CourtClient {
    cl: reqwest::Client,
}

impl CourtClient {
    pub fn new() -> Self {
        Self {
            cl: reqwest::Client::new(),
        }
    }
}

impl ServiceTraitBounds for CourtClient {}

#[cfg(test)]
impl ServiceTraitBounds for MockCourtClientApi {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CourtClientApi for CourtClient {
    async fn share_with_court(
        &self,
        court_url: &url::Url,
        bill_to_share: BillToShareWithExternalParty,
        sharer_keys: &BcrKeys,
    ) -> Result<()> {
        let request = ReceiveBillRequest {
            content: bill_to_share,
            public_key: sharer_keys.pub_key(),
        };
        let serialized = to_vec(&request).map_err(Error::Borsh)?;
        let hash = Sha256::hash(&serialized);
        let msg = Message::from_digest(*hash.as_ref());
        let signature = SECP256K1.sign_schnorr(&msg, &sharer_keys.get_key_pair());

        let signed_req = SignedReceiveBillRequest { request, signature };

        let url = court_url.join("/v1/bill/receive").expect("is a valid url");

        self.cl
            .post(url)
            .json(&signed_req)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
