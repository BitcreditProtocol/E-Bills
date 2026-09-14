use bitcoin::secp256k1::{PublicKey, SecretKey};
use thiserror::Error;

mod bcrkeys;
pub mod btc;

pub type Result<T> = std::result::Result<T, Error>;
pub use bcrkeys::BcrKeys;
pub use bcrkeys::DeriveKeypair;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Private key error: {0}")]
    PrivateKey(#[from] bitcoin::secp256k1::Error),

    #[error("Ecies encryption error: {0}")]
    Ecies(String),

    #[error("Aggregated signature needs at least 2 keys")]
    TooFewKeys,

    /// Errors stemming from decoding base58
    #[error("Decode base58 error: {0}")]
    Decode(#[from] bitcoin::base58::InvalidCharacterError),

    /// Errors stemming from parsing the recovery id
    #[error("Parse recovery id error: {0}")]
    ParseRecoveryId(#[from] std::num::ParseIntError),

    /// all errors originating from dealing with bitcoin descriptors
    #[error("Bitcoin Descriptor error: {0}")]
    BtcDescriptor(String),

    /// all errors originating from dealing with bitcoin addresses
    #[error("Bitcoin Address error: {0}")]
    BtcAddress(String),

    /// all errors originating from dealing with tweaks
    #[error("External Bitcoin Tweak error: {0}")]
    Tweak(String),

    #[error("Mnemonic seed phrase error {0}")]
    Mnemonic(#[from] bip39::Error),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Signature error: {0}")]
    Signature(String),
}

/// Encrypt the given bytes with the given Secp256k1 key via ECIES
pub fn encrypt_ecies(bytes: &[u8], public_key: &PublicKey) -> Result<Vec<u8>> {
    let pub_key_bytes = public_key.serialize();
    let encrypted =
        ecies::encrypt(pub_key_bytes.as_slice(), bytes).map_err(|e| Error::Ecies(e.to_string()))?;
    Ok(encrypted)
}

/// Decrypt the given bytes with the given Secp256k1 key via ECIES
pub fn decrypt_ecies(bytes: &[u8], private_key: &SecretKey) -> Result<Vec<u8>> {
    let decrypted = ecies::decrypt(private_key.secret_bytes().as_slice(), bytes)
        .map_err(|e| Error::Ecies(e.to_string()))?;
    Ok(decrypted)
}
