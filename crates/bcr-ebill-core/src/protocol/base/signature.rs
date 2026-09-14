use bitcoin::base58;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, str::FromStr};

use bitcoin::secp256k1::{Keypair, Message, PublicKey, SECP256K1, Scalar, SecretKey, schnorr};

use crate::protocol::{ProtocolValidationError, Sha256Hash, crypto::Error as CryptoError};

/// Type for representing a base58-encoded schnorr Signature
#[derive(Debug, Clone, Eq, Serialize, Deserialize, PartialEq, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct SchnorrSignature(String);

impl SchnorrSignature {
    pub fn new(s: &str) -> Result<Self, ProtocolValidationError> {
        Self::from_str(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_sig(&self) -> schnorr::Signature {
        // safe, since we validate this to be a base58 encoded schnorr signature
        let decoded = base58::decode(&self.0).expect("is base58 encoded");
        schnorr::Signature::from_slice(&decoded).expect("is a valid schnorr signature")
    }

    pub fn sign(hash: &Sha256Hash, private_key: &SecretKey) -> Result<Self, CryptoError> {
        let key_pair = Keypair::from_secret_key(SECP256K1, private_key);
        let msg = Message::from_digest_slice(&hash.decode())
            .map_err(|e| CryptoError::Signature(e.to_string()))?;
        let signature = SECP256K1.sign_schnorr(&msg, &key_pair);
        Ok(Self::from(signature))
    }

    pub fn sign_aggregated(hash: &Sha256Hash, keys: &[SecretKey]) -> Result<Self, CryptoError> {
        if keys.len() < 2 {
            return Err(CryptoError::Signature(
                "too few keys for aggregated signature".to_string(),
            ));
        }

        let first_key = keys.first().expect("keys length checked above to be >= 2");
        let mut aggregated_key: SecretKey = first_key.to_owned();
        for key in keys.iter().skip(1) {
            aggregated_key = aggregated_key
                .add_tweak(&Scalar::from(*key))
                .map_err(|e| CryptoError::Signature(e.to_string()))?;
        }

        let aggregated_key_pair = Keypair::from_secret_key(SECP256K1, &aggregated_key);
        let msg = Message::from_digest_slice(&hash.decode())
            .map_err(|e| CryptoError::Signature(e.to_string()))?;
        let signature = SECP256K1.sign_schnorr(&msg, &aggregated_key_pair);

        Ok(Self::from(signature))
    }

    pub fn verify(&self, hash: &Sha256Hash, public_key: &PublicKey) -> Result<bool, CryptoError> {
        let (pub_key, _) = public_key.x_only_public_key();
        let msg = Message::from_digest_slice(&hash.decode())
            .map_err(|e| CryptoError::Signature(e.to_string()))?;
        let decoded_signature = self.as_sig();
        Ok(SECP256K1
            .verify_schnorr(&decoded_signature, &msg, &pub_key)
            .is_ok())
    }

    /// Returns the combined public key for the given public keys
    pub fn combine_pub_keys(pub_keys: &[PublicKey]) -> Result<PublicKey, CryptoError> {
        if pub_keys.len() < 2 {
            return Err(CryptoError::Signature(
                "too few keys for aggregated signature".to_string(),
            ));
        }

        let combined_key = PublicKey::combine_keys(&pub_keys.iter().collect::<Vec<&PublicKey>>())?;
        Ok(combined_key)
    }

    /// Returns the aggregated public key for the given private keys
    pub fn get_aggregated_public_key(private_keys: &[SecretKey]) -> Result<PublicKey, CryptoError> {
        if private_keys.len() < 2 {
            return Err(CryptoError::Signature(
                "too few keys for aggregated signature".to_string(),
            ));
        }

        let key_pairs: Vec<Keypair> = private_keys
            .iter()
            .map(|private_key| Keypair::from_secret_key(SECP256K1, private_key))
            .collect::<Vec<Keypair>>();
        let public_keys: Vec<PublicKey> = key_pairs.into_iter().map(|kp| kp.public_key()).collect();

        let first_key = public_keys.first().expect("keys length checked above");
        let mut aggregated_key: PublicKey = first_key.to_owned();
        for key in public_keys.iter().skip(1) {
            aggregated_key = aggregated_key.combine(key)?;
        }
        Ok(aggregated_key)
    }
}

impl Display for SchnorrSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<schnorr::Signature> for SchnorrSignature {
    fn from(value: schnorr::Signature) -> Self {
        SchnorrSignature(base58::encode(&value.serialize()))
    }
}

impl FromStr for SchnorrSignature {
    type Err = ProtocolValidationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let decoded = base58::decode(s).map_err(|_| ProtocolValidationError::InvalidSignature)?;
        schnorr::Signature::from_slice(&decoded)
            .map_err(|_| ProtocolValidationError::InvalidSignature)?;
        Ok(SchnorrSignature(s.to_owned()))
    }
}

impl TryFrom<String> for SchnorrSignature {
    type Error = ProtocolValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<SchnorrSignature> for String {
    fn from(value: SchnorrSignature) -> Self {
        value.0
    }
}

impl borsh::BorshSerialize for SchnorrSignature {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        borsh::BorshSerialize::serialize(&self.0, writer)
    }
}

impl borsh::BorshDeserialize for SchnorrSignature {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let sig_str: String = borsh::BorshDeserialize::deserialize_reader(reader)?;
        SchnorrSignature::new(&sig_str)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::crypto::BcrKeys;

    use super::*;
    use borsh::BorshDeserialize;
    use serde::{Deserialize, Serialize};

    const VALID_SIG: &str =
        "23u7iXhvpRBYhdHQW3jEk5LyQWJGDcnCoCfiPvjPHXQqmun6z3ZrYX7eXMrBmZk4mHW4Y5DQbASJb1LZU1KrkgGH";

    #[derive(
        Debug,
        Clone,
        Eq,
        PartialEq,
        borsh_derive::BorshSerialize,
        borsh_derive::BorshDeserialize,
        Serialize,
        Deserialize,
    )]
    pub struct TestSig {
        pub sig: SchnorrSignature,
    }

    #[test]
    fn test_serialization() {
        let sig = SchnorrSignature::new(VALID_SIG).expect("works");
        let test = TestSig { sig: sig.clone() };
        let json = serde_json::to_string(&test).unwrap();
        assert_eq!(
            "{\"sig\":\"23u7iXhvpRBYhdHQW3jEk5LyQWJGDcnCoCfiPvjPHXQqmun6z3ZrYX7eXMrBmZk4mHW4Y5DQbASJb1LZU1KrkgGH\"}",
            json
        );
        let deserialized = serde_json::from_str(&json).unwrap();
        assert_eq!(test, deserialized);
        assert_eq!(sig, deserialized.sig);

        let borsh = borsh::to_vec(&sig).unwrap();
        let borsh_de = SchnorrSignature::try_from_slice(&borsh).unwrap();
        assert_eq!(sig, borsh_de);

        let borsh_test = borsh::to_vec(&test).unwrap();
        let borsh_de_test = TestSig::try_from_slice(&borsh_test).unwrap();
        assert_eq!(test, borsh_de_test);
        assert_eq!(sig, borsh_de_test.sig);
    }

    #[test]
    fn test_invalid_serde_serialization() {
        let json = "{\"sig\":\"invalid\"}";
        let deserialized = serde_json::from_str::<TestSig>(json);
        assert!(deserialized.is_err());

        let borsh = borsh::to_vec(&String::from("invalid")).expect("works");
        let res = SchnorrSignature::try_from_slice(&borsh);
        assert!(res.is_err());

        let borsh = borsh::to_vec(VALID_SIG).expect("works");
        let res = SchnorrSignature::try_from_slice(&borsh);
        assert!(res.is_ok());
    }

    #[test]
    fn test_sig() {
        let n = SchnorrSignature::from_str(VALID_SIG).expect("works");
        let n_owned: SchnorrSignature = String::from(VALID_SIG).try_into().expect("works");
        assert_eq!(n, n_owned);

        assert!(matches!(
            SchnorrSignature::new("blablub"),
            Err(ProtocolValidationError::InvalidSignature)
        ));
        assert!(matches!(
            SchnorrSignature::new(
                "ABAB23u7iXhvpRBYhdHQW3jEk5LyQWJGDcnCoCfiPvjPHXQqmun6z3ZrYX7eXMrBmZk4mHW4Y5DQbASJb1LZU1KrkgGH"
            ),
            Err(ProtocolValidationError::InvalidSignature)
        ));
    }

    #[test]
    fn test_sign_verify() {
        let keypair = BcrKeys::new();
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let signature = SchnorrSignature::sign(&hash, &keypair.get_private_key()).unwrap();
        assert!(signature.verify(&hash, &keypair.pub_key()).is_ok());
        assert!(signature.verify(&hash, &keypair.pub_key()).unwrap());
    }

    #[test]
    fn test_sign_verify_invalid() {
        let keypair = BcrKeys::new();
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let signature = SchnorrSignature::sign(&hash, &keypair.get_private_key()).unwrap();
        let hash2 = Sha256Hash::from_bytes("Hello, Changed Changed Changed World".as_bytes());
        assert!(signature.verify(&hash, &keypair.pub_key()).is_ok());
        assert!(signature.verify(&hash, &keypair.pub_key()).is_ok());
        // it fails for a different hash
        assert!(signature.verify(&hash2, &keypair.pub_key()).is_ok());
        assert!(
            !signature
                .verify(&hash2, &keypair.pub_key())
                .as_ref()
                .unwrap()
        );
    }

    #[test]
    fn test_sign_verify_aggregated() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());

        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        assert!(signature.verify(&hash, &public_key).is_ok());
        assert!(signature.verify(&hash, &public_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_order_dependence() {
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());

        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();

        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let keys2: Vec<SecretKey> = vec![keypair2.get_private_key(), keypair1.get_private_key()];
        let public_key2 = SchnorrSignature::get_aggregated_public_key(&keys2).unwrap();
        let signature2 = SchnorrSignature::sign_aggregated(&hash, &keys2).unwrap();

        assert_ne!(signature, signature2); // the signatures aren't the same
        assert_eq!(public_key, public_key2); // but the public keys are

        assert!(signature.verify(&hash, &public_key).is_ok());
        assert!(signature.verify(&hash, &public_key).unwrap());

        assert!(signature2.verify(&hash, &public_key2).is_ok());
        assert!(signature2.verify(&hash, &public_key2).unwrap());

        assert!(signature.verify(&hash, &public_key2).is_ok());
        assert!(signature.verify(&hash, &public_key2).unwrap());

        assert!(signature2.verify(&hash, &public_key).is_ok());
        assert!(signature2.verify(&hash, &public_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_invalid() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());

        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let changed_hash = Sha256Hash::from_bytes("Hello Hello, World".as_bytes());
        assert!(signature.verify(&changed_hash, &public_key).is_ok());
        assert!(!signature.verify(&changed_hash, &public_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_invalid_only_one_pubkey() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();
        assert!(signature.verify(&hash, &keypair2.pub_key()).is_ok());
        assert!(!signature.verify(&hash, &keypair2.pub_key()).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_multiple() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keypair3 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![
            keypair1.get_private_key(),
            keypair2.get_private_key(),
            keypair3.get_private_key(),
        ];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();
        assert!(signature.verify(&hash, &public_key).is_ok());
        assert!(signature.verify(&hash, &public_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_multiple_manually_combined_pubkeys_for_verify() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keypair3 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![
            keypair1.get_private_key(),
            keypair2.get_private_key(),
            keypair3.get_private_key(),
        ];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let combined_pub_key = keypair1
            .get_key_pair()
            .public_key()
            .combine(&keypair2.get_key_pair().public_key())
            .unwrap()
            .combine(&keypair3.get_key_pair().public_key())
            .unwrap();

        assert_eq!(public_key, combined_pub_key);
        assert!(signature.verify(&hash, &combined_pub_key).is_ok());
        assert!(signature.verify(&hash, &combined_pub_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_manually_combined_pubkeys_for_verify() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let combined_pub_key = keypair1
            .get_key_pair()
            .public_key()
            .combine(&keypair2.get_key_pair().public_key())
            .unwrap();

        assert_eq!(public_key, combined_pub_key);
        assert!(signature.verify(&hash, &combined_pub_key).is_ok());
        assert!(signature.verify(&hash, &combined_pub_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_manually_combined_pubkeys_for_verify_different_order_also_works()
    {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let combined_pub_key = keypair2
            .get_key_pair()
            .public_key()
            .combine(&keypair1.get_key_pair().public_key())
            .unwrap();
        assert_eq!(public_key, combined_pub_key);
        assert!(signature.verify(&hash, &combined_pub_key).is_ok());
        assert!(signature.verify(&hash, &combined_pub_key).unwrap());
    }

    #[test]
    fn test_sign_verify_aggregated_manually_combined_pubkeys_for_verify_invalid_pubkey() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keypair3 = BcrKeys::new();
        let keys: Vec<SecretKey> = vec![keypair1.get_private_key(), keypair2.get_private_key()];
        let hash = Sha256Hash::from_bytes("Hello, World".as_bytes());
        let public_key = SchnorrSignature::get_aggregated_public_key(&keys).unwrap();
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys).unwrap();

        let combined_pub_key =
            SchnorrSignature::combine_pub_keys(&[keypair1.pub_key(), keypair3.pub_key()]).unwrap();
        let combined_correct_pub_key =
            SchnorrSignature::combine_pub_keys(&[keypair1.pub_key(), keypair2.pub_key()]).unwrap();
        assert_ne!(public_key, combined_pub_key);
        assert_eq!(public_key, combined_correct_pub_key);
        assert!(signature.verify(&hash, &combined_pub_key).is_ok());
        assert!(signature.verify(&hash, &combined_correct_pub_key).is_ok());
        assert!(!signature.verify(&hash, &combined_pub_key).unwrap());
    }

    #[test]
    fn test_combine_pub_keys_baseline() {
        let keypair1 = BcrKeys::new();
        let keypair2 = BcrKeys::new();
        let keypair3 = BcrKeys::new();
        let combined_pub_key = SchnorrSignature::combine_pub_keys(&[
            keypair1.pub_key(),
            keypair2.pub_key(),
            keypair3.pub_key(),
        ])
        .unwrap();
        let combined_pub_key_diff_order = SchnorrSignature::combine_pub_keys(&[
            keypair2.pub_key(),
            keypair1.pub_key(),
            keypair3.pub_key(),
        ])
        .unwrap();
        // when combining public keys, the order isn't important
        assert_eq!(combined_pub_key, combined_pub_key_diff_order);
    }

    #[test]
    fn test_combine_pub_keys_too_few() {
        let keypair1 = BcrKeys::new();
        let combined_pub_key = SchnorrSignature::combine_pub_keys(&[keypair1.pub_key()]);
        assert!(combined_pub_key.is_err());
    }
}
