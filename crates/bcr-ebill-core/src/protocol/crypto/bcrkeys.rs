use super::Result;
use std::str::FromStr;

use crate::protocol::blockchain::BlockchainType;

use bip39::Mnemonic;
use bitcoin::{
    Network,
    hashes::{Hash, HashEngine, Hmac, HmacEngine, sha256, sha512},
    secp256k1::{Keypair, PublicKey, SECP256K1, SecretKey, rand},
};

/// Number of words to use when generating BIP39 seed phrases
const BIP39_WORD_COUNT: usize = 12;

/// A wrapper around the secp256k1 keypair that can be used for
/// Bitcoin and Nostr keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BcrKeys {
    inner: Keypair,
}

impl BcrKeys {
    /// Generates a fresh random keypair that can be used for
    /// Bitcoin and Nostr keys.
    pub fn new() -> Self {
        Self {
            inner: Keypair::new(SECP256K1, &mut rand::thread_rng()),
        }
    }

    /// Generates a fresh random keypair including a seed phrase that
    /// can be used to recover the private_key
    pub fn new_with_seed_phrase() -> Result<(Self, String)> {
        let (seed_phrase, keypair) = generate_keypair_from_seed_phrase()?;
        let keys = Self { inner: keypair };
        Ok((keys, seed_phrase.to_string()))
    }

    /// Recovers keys from a given seedphrase
    pub fn from_seedphrase(seed: &str) -> Result<Self> {
        let recovered_keys = keypair_from_seed_phrase(seed)?;
        Ok(Self {
            inner: recovered_keys,
        })
    }

    /// Loads a keypair from a given private key string
    pub fn from_private_key(private_key: &SecretKey) -> Self {
        let keypair = Keypair::from_secret_key(SECP256K1, private_key);
        Self { inner: keypair }
    }

    pub fn from_private_key_string(private_key: &str) -> Result<Self> {
        let private_key = SecretKey::from_str(private_key)?;
        Ok(Self::from_private_key(&private_key))
    }

    /// Returns the private key as a hex encoded string
    pub fn get_private_key_string(&self) -> String {
        self.inner.secret_key().display_secret().to_string()
    }

    /// Returns the private key
    pub fn get_private_key(&self) -> SecretKey {
        self.inner.secret_key()
    }

    pub fn pub_key(&self) -> PublicKey {
        self.inner.public_key()
    }

    /// Returns the public key as a hex encoded string
    pub fn get_public_key(&self) -> String {
        self.inner.public_key().to_string()
    }

    pub fn get_bitcoin_keys(
        &self,
        used_network: Network,
    ) -> (bitcoin::PrivateKey, bitcoin::PublicKey) {
        let private_key = self.get_bitcoin_private_key(used_network);
        (private_key, private_key.public_key(SECP256K1))
    }

    /// Returns the key pair as a bitcoin private key for the given network
    pub fn get_bitcoin_private_key(&self, used_network: Network) -> bitcoin::PrivateKey {
        bitcoin::PrivateKey::new(self.inner.secret_key(), used_network)
    }

    /// Returns the key pair as a nostr key pair
    pub fn get_nostr_keys(&self) -> nostr::key::Keys {
        let nostr_key = nostr::key::SecretKey::from_slice(&self.inner.secret_key().secret_bytes())
            .expect("valid secret key");
        nostr::key::Keys::new(nostr_key)
    }

    /// Returns the secp256k1 key pair
    pub fn get_key_pair(&self) -> Keypair {
        self.inner
    }

    /// Derives a keypair from the private key, using a chain type and an index.
    pub fn derive_chain_keypair(&self, chain_type: BlockchainType, index: u32) -> Result<Keypair> {
        derive_keypair(&self.get_private_key(), chain_type, index)
    }
}

impl Default for BcrKeys {
    fn default() -> Self {
        Self::new()
    }
}

pub trait DeriveKeypair {
    /// Derives the first identity keypair with a chain type from context.
    fn derive_identity_keypair(&self) -> Result<Keypair>;
    /// Derives the first company keypair with a chain type from context.
    fn derive_company_keypair(&self) -> Result<Keypair>;
    /// Derives the first bill keypair with a chain type from context.
    fn derive_bill_keypair(&self) -> Result<Keypair>;
}

impl DeriveKeypair for BcrKeys {
    fn derive_identity_keypair(&self) -> Result<Keypair> {
        derive_keypair(&self.get_private_key(), BlockchainType::Identity, 0)
    }

    fn derive_company_keypair(&self) -> Result<Keypair> {
        derive_keypair(&self.get_private_key(), BlockchainType::Company, 0)
    }

    fn derive_bill_keypair(&self) -> Result<Keypair> {
        derive_keypair(&self.get_private_key(), BlockchainType::Bill, 0)
    }
}

/// Generate a new secp256k1 keypair using a 12 word seed phrase.
/// Returns both the keypair and the Mnemonic with the seed phrase.
fn generate_keypair_from_seed_phrase() -> Result<(Mnemonic, Keypair)> {
    let mnemonic = Mnemonic::generate(BIP39_WORD_COUNT)?;
    let keypair = keypair_from_mnemonic(&mnemonic)?;
    Ok((mnemonic, keypair))
}

/// Recover a key pair from a BIP39 seed phrase. Word count
/// and language are detected automatically.
fn keypair_from_seed_phrase(words: &str) -> Result<Keypair> {
    let mnemonic = Mnemonic::from_str(words)?;
    keypair_from_mnemonic(&mnemonic)
}

/// Recover a key pair from a BIP39 mnemonic.
fn keypair_from_mnemonic(mnemonic: &Mnemonic) -> Result<Keypair> {
    let seed = mnemonic.to_seed("");
    let (key, _) = seed.split_at(32);
    let secret = SecretKey::from_slice(key)?;
    Ok(Keypair::from_secret_key(SECP256K1, &secret))
}

/// Allows us to derive a keypair from a parent key, using a chain type and an index. This is
/// similar to BIP32 but with different chain types and using priv instead of xpriv keys.
fn derive_keypair(
    parent_key: &SecretKey,
    chain_type: BlockchainType,
    index: u32,
) -> Result<Keypair> {
    let chain_type_hash = sha256::Hash::hash(chain_type.to_string().into_bytes().as_slice());
    let mut msg = Vec::with_capacity(4 + 32);
    msg.extend_from_slice(&index.to_be_bytes());
    msg.extend_from_slice(&chain_type_hash.to_byte_array());

    let mut mac: HmacEngine<sha512::Hash> = HmacEngine::new(parent_key.secret_bytes().as_slice());
    mac.input(&msg);
    let hmac_result: Hmac<sha512::Hash> = Hmac::from_engine(mac);
    let secret_key = SecretKey::from_slice(&hmac_result[..32])?;
    Ok(Keypair::from_secret_key(SECP256K1, &secret_key))
}

#[cfg(test)]
mod tests {
    use nostr::nips::nip19::ToBech32;

    use super::*;
    use crate::protocol::crypto::{decrypt_ecies, encrypt_ecies};

    fn priv_key() -> SecretKey {
        SecretKey::from_str("926a7ce0fdacad199307bcbbcda4869bca84d54b939011bafe6a83cb194130d3")
            .unwrap()
    }

    #[test]
    fn test_derive_keypair() {
        let expected =
            SecretKey::from_str("11bd676b4231ebd549fa12c0acf62a415b3f88b1ed110c9cc2c6f63ac4f85667")
                .unwrap();
        let keypair = derive_keypair(&priv_key(), BlockchainType::Bill, 0).unwrap();
        assert_eq!(
            keypair.secret_key(),
            expected,
            "keypair should be derived deterministically"
        );

        let expected_company =
            SecretKey::from_str("5807c32decb208ba8d1647f9b6bfd39d744c1999f7e06594a8d41dd5ff542a68")
                .unwrap();
        let keypair_company = derive_keypair(&priv_key(), BlockchainType::Company, 0).unwrap();
        assert!(
            keypair_company.secret_key() != keypair.secret_key(),
            "keypair should be different for different chains"
        );
        assert_eq!(
            keypair_company.secret_key(),
            expected_company,
            "company keypair should be derived deterministically"
        );

        let expected2 =
            SecretKey::from_str("6cc0e196571db869d88857fdaa94fa54ca69b4339d0e5cdf2e4ff7f5cf4fe5f7")
                .unwrap();
        let keypair2 = derive_keypair(&priv_key(), BlockchainType::Bill, 1).unwrap();
        assert_eq!(
            keypair2.secret_key(),
            expected2,
            "keypair should change with different index"
        );
    }

    #[test]
    fn test_derive_keypair_from_bcr_keys() {
        let expected =
            SecretKey::from_str("11bd676b4231ebd549fa12c0acf62a415b3f88b1ed110c9cc2c6f63ac4f85667")
                .unwrap();
        let bcr_keys = BcrKeys::from_private_key(&priv_key());
        let keypair = bcr_keys
            .derive_chain_keypair(BlockchainType::Bill, 0)
            .expect("could not derive keypair");
        assert_eq!(
            keypair.secret_key(),
            expected,
            "keypair should be derived deterministically"
        );
    }

    #[test]
    fn test_derive_keypair_from_bill_keys() {
        let expected =
            SecretKey::from_str("11bd676b4231ebd549fa12c0acf62a415b3f88b1ed110c9cc2c6f63ac4f85667")
                .unwrap();
        let bcr_keys = BcrKeys::from_private_key(&priv_key());
        let bill_keys = BcrKeys::from_private_key(&bcr_keys.get_private_key());
        let keypair = bill_keys
            .derive_bill_keypair()
            .expect("could not derive keypair from bill keys");
        assert_eq!(keypair.secret_key(), expected);
    }

    #[test]
    fn test_derive_keypair_from_company_keys() {
        let expected =
            SecretKey::from_str("5807c32decb208ba8d1647f9b6bfd39d744c1999f7e06594a8d41dd5ff542a68")
                .unwrap();
        let bcr_keys = BcrKeys::from_private_key(&priv_key());
        let company_keys = BcrKeys::from_private_key(&bcr_keys.get_private_key());
        let keypair = company_keys
            .derive_company_keypair()
            .expect("could not derive keypair from company keys");
        assert_eq!(keypair.secret_key(), expected);
    }

    #[test]
    fn test_generate_keypair_and_seed_phrase_round_trip() {
        let (mnemonic, keypair) = generate_keypair_from_seed_phrase()
            .expect("Could not generate keypair and seed phrase");
        let recovered_keys = keypair_from_seed_phrase(&mnemonic.to_string())
            .expect("Could not recover private key from seed phrase");
        assert_eq!(keypair.secret_key(), recovered_keys.secret_key());
    }

    #[test]
    fn test_recover_keypair_from_seed_phrase_24_words() {
        // a valid pair of 24 words to priv key
        let words = "forward paper connect economy twelve debate cart isolate accident creek bind predict captain rifle glory cradle hip whisper wealth save buddy place develop dolphin";
        let priv_key = "f31e0373f6fa9f4835d49a278cd48f47ea115af7480edf435275a3c2dbb1f982";
        let keypair =
            keypair_from_seed_phrase(words).expect("Could not create keypair from seed phrase");
        let returned_priv_key = keypair.secret_key().display_secret().to_string();
        assert_eq!(priv_key, returned_priv_key);
    }

    #[test]
    fn test_recover_keypair_from_seed_phrase_12_words() {
        // a valid pair of 12 words to priv key
        let words = "oblige repair kind park dust act name myth cheap treat hammer arrive";
        let priv_key = "92f920d8e183cab62723c3a7eee9cb0b3edb3c4aad459f4062cfb7960b570662";
        let keypair =
            keypair_from_seed_phrase(words).expect("Could not create keypair from seed phrase");
        let returned_priv_key = keypair.secret_key().display_secret().to_string();
        assert_eq!(priv_key, returned_priv_key);
    }

    #[test]
    fn test_new_keypair() {
        let keypair = BcrKeys::new();
        assert!(!keypair.get_private_key_string().is_empty());
        assert!(!keypair.get_public_key().is_empty());
        assert!(
            !keypair
                .get_bitcoin_private_key(Network::Bitcoin)
                .to_string()
                .is_empty()
        );
        assert!(keypair.get_nostr_keys().public_key().to_bech32().is_ok());
    }

    #[test]
    fn test_load_keypair() {
        let keypair = BcrKeys::from_private_key(&priv_key());
        let keypair2 = BcrKeys::from_private_key(&priv_key());
        assert_eq!(
            keypair.get_private_key_string(),
            keypair2.get_private_key_string()
        );
        assert_eq!(keypair.get_public_key(), keypair2.get_public_key());
        assert_eq!(
            keypair.get_bitcoin_private_key(Network::Bitcoin),
            keypair2.get_bitcoin_private_key(Network::Bitcoin)
        );
        assert_eq!(keypair.get_nostr_keys(), keypair2.get_nostr_keys());
    }

    #[test]
    fn encrypt_decrypt_ecies_basic() {
        let msg = "Hello, this is a very important message!"
            .to_string()
            .into_bytes();
        let keypair = BcrKeys::new();

        let encrypted = encrypt_ecies(&msg, &keypair.pub_key());
        assert!(encrypted.is_ok());
        let decrypted = decrypt_ecies(encrypted.as_ref().unwrap(), &keypair.get_private_key());
        assert!(decrypted.is_ok());

        assert_eq!(&msg, decrypted.as_ref().unwrap());
    }

    #[test]
    fn encrypt_decrypt_with_derived_key() {
        let msg = "Hello, this is a very important message!"
            .to_string()
            .into_bytes();
        let keypair = BcrKeys::new()
            .derive_chain_keypair(BlockchainType::Identity, 0)
            .expect("Failed to derive identity keypair");

        let encrypted = encrypt_ecies(&msg, &keypair.public_key());
        assert!(encrypted.is_ok());
        let decrypted = decrypt_ecies(encrypted.as_ref().unwrap(), &keypair.secret_key());
        assert!(decrypted.is_ok());

        assert_eq!(&msg, decrypted.as_ref().unwrap());
    }

    #[test]
    fn encrypt_decrypt_ecies_hardcoded_creds() {
        let msg = "Important!".to_string().into_bytes();

        let encrypted = encrypt_ecies(
            &msg,
            &PublicKey::from_str(
                "03205b8dec12bc9e879f5b517aa32192a2550e88adcee3e54ec2c7294802568fef",
            )
            .unwrap(),
        );
        assert!(encrypted.is_ok());
        let decrypted = decrypt_ecies(
            encrypted.as_ref().unwrap(),
            &SecretKey::from_str(
                "8863c82829480536893fc49c4b30e244f97261e989433373d73c648c1a656a79",
            )
            .unwrap(),
        );
        assert!(decrypted.is_ok());

        assert_eq!(&msg, decrypted.as_ref().unwrap());
    }

    #[test]
    fn get_key_pair() {
        let keys = BcrKeys::new();
        let key_pair = keys.get_key_pair();
        let pub_key = key_pair.public_key().to_string();
        assert_eq!(keys.get_public_key(), pub_key);
    }
}
