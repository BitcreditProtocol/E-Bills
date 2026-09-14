use async_trait::async_trait;
use bcr_common::cashu::{self, State, nut01 as cdk01, nut02 as cdk02};
use bcr_common::client::mint::Client as ExternalMintClient;
use bcr_common::core::NodeId;
use bcr_common::core::{BillId, keys::to_fee_and_amounts};
use bcr_common::ecash::{self, ProofsMethods};
use bcr_common::wallet;
use bcr_common::wire::quotes::{ResolveOffer, StatusReply};
use bcr_ebill_core::protocol::{BitcoinAddress, BlockId, Sha256Hash, Sum};
use bcr_ebill_core::{
    application::ServiceTraitBounds, protocol::DateTimeUtc, protocol::SecretKey,
    protocol::blockchain::bill::BillToShareWithExternalParty, protocol::crypto::BcrKeys,
};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::rand::{prelude::SliceRandom, thread_rng};
use std::ops::Deref;
use std::str::FromStr;
use thiserror::Error;
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
    /// Look up a quote for a mint
    async fn lookup_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
    ) -> Result<QuoteStatusReply>;
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

    async fn lookup_quote_for_mint(
        &self,
        mint_url: &url::Url,
        quote_id: &Uuid,
    ) -> Result<QuoteStatusReply> {
        let reply = self
            .client(mint_url)?
            .lookup(quote_id.to_owned())
            .await
            .map_err(|e| {
                log::error!("Error looking up request on mint {mint_url}: {e}");
                Error::QuoteClient
            })?;
        Ok(reply.into())
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
