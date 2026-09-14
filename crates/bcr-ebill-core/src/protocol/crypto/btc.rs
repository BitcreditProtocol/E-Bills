use std::{fmt::Display, str::FromStr};

use super::{Error, Result};
use crate::protocol::{
    BitcoinAddress, BlockId, ProtocolValidationError, PublicKey, Sha256Hash,
    blockchain::bill::BillOpCode,
};
use bitcoin::secp256k1::{SECP256K1, Scalar};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Hash)]
#[serde(try_from = "String", into = "String")]
pub struct BtcDescriptor(String);

impl BtcDescriptor {
    pub fn new(desc: impl Into<String>) -> std::result::Result<Self, ProtocolValidationError> {
        let s = desc.into();
        miniscript::Descriptor::<miniscript::descriptor::DescriptorPublicKey>::parse_descriptor(
            SECP256K1, &s,
        )
        .map_err(|_| ProtocolValidationError::InvalidBitcoinDescriptor)?;

        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for BtcDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for BtcDescriptor {
    type Err = ProtocolValidationError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        BtcDescriptor::new(s)
    }
}

impl TryFrom<String> for BtcDescriptor {
    type Error = ProtocolValidationError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<BtcDescriptor> for String {
    fn from(value: BtcDescriptor) -> Self {
        value.0
    }
}

impl borsh::BorshSerialize for BtcDescriptor {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        borsh::BorshSerialize::serialize(&self.0, writer)
    }
}

impl borsh::BorshDeserialize for BtcDescriptor {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let s: String = borsh::BorshDeserialize::deserialize_reader(reader)?;
        BtcDescriptor::new(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Get p2tr address for the given keys
pub fn get_address_to_pay(
    bill_public_key: &PublicKey,
    holder_public_key: &PublicKey,
    tweak_hash: &Sha256Hash,
    network: bitcoin::Network,
) -> Result<BitcoinAddress> {
    let combined_key = bill_public_key
        .combine(holder_public_key)
        .map_err(Error::from)?;

    // tweak key with the given tweak hash
    let tweak = Scalar::from_be_bytes(tweak_hash.decode_to_array())
        .map_err(|e| Error::Tweak(e.to_string()))?;
    let tweaked_key = combined_key.add_exp_tweak(bitcoin::secp256k1::global::SECP256K1, &tweak)?;

    let (x_only_pub_key, _parity) = tweaked_key.x_only_public_key();

    Ok(bitcoin::Address::p2tr(
        bitcoin::secp256k1::global::SECP256K1,
        x_only_pub_key,
        None,
        network,
    )
    .as_unchecked()
    .to_owned())
}

/// Get tr descriptor with wif for the given keys
pub fn get_combined_private_descriptor(
    pkey: &bitcoin::PrivateKey,
    pkey_to_combine: &bitcoin::PrivateKey,
    tweak_hash: &Sha256Hash,
    network: bitcoin::Network,
) -> Result<BtcDescriptor> {
    let combined_key = pkey.inner.add_tweak(&Scalar::from(pkey_to_combine.inner))?;

    // tweak key with the given tweak hash
    let tweak = Scalar::from_be_bytes(tweak_hash.decode_to_array())
        .map_err(|e| Error::Tweak(e.to_string()))?;
    let tweaked_key = combined_key.add_tweak(&tweak)?;

    let priv_key = bitcoin::PrivateKey::new(tweaked_key, network);
    let single = miniscript::descriptor::SinglePriv {
        key: priv_key,
        origin: None,
    };
    let desc_seckey = miniscript::descriptor::DescriptorSecretKey::Single(single);
    let desc_pubkey = desc_seckey
        .to_public(bitcoin::secp256k1::global::SECP256K1)
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?;
    let kmap = miniscript::descriptor::KeyMap::from_iter(std::iter::once((
        desc_pubkey.clone(),
        desc_seckey,
    )));
    let desc = miniscript::Descriptor::new_tr(desc_pubkey, None)
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?;
    let btc_desc = BtcDescriptor::new(desc.to_string_with_secret(&kmap))
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?;

    Ok(btc_desc)
}

/// Parses a given string combined private descriptor to a descriptor, pub key and bitcoin address
pub fn parse_private_descriptor(
    private_desc: &BtcDescriptor,
    network: bitcoin::Network,
) -> Result<(
    miniscript::Descriptor<miniscript::descriptor::DescriptorPublicKey>,
    bitcoin::PrivateKey,
    BitcoinAddress,
)> {
    let (desc, key_map) =
        miniscript::Descriptor::<miniscript::descriptor::DescriptorPublicKey>::parse_descriptor(
            SECP256K1,
            private_desc.as_str(),
        )
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?;

    let internal_key = match &desc {
        miniscript::Descriptor::Tr(tr) => tr.internal_key(),
        _ => return Err(Error::BtcDescriptor("Not a Taproot descriptor".to_string())),
    };

    let secret_key = key_map
        .get(internal_key)
        .ok_or_else(|| Error::BtcDescriptor("No secret key".to_string()))?;

    let privkey = match secret_key {
        miniscript::descriptor::DescriptorSecretKey::Single(single) => single.key,
        _ => return Err(Error::BtcDescriptor("Invalid Secret Key".to_string())),
    };

    let address = desc
        .derived_descriptor(SECP256K1, 0)
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?
        .address(network)
        .map_err(|e| Error::BtcDescriptor(e.to_string()))?;

    Ok((desc, privkey, address.into_unchecked()))
}

/// Calculates the tweak for the payment address with the previous hash, the block id and a tag based on the
/// type of payment
pub fn calculate_tweak_hash_for_payment_request(
    payment_op: BillOpCode,
    block_id: &BlockId,
    previous_hash: &Sha256Hash,
) -> Result<Sha256Hash> {
    let mut input = Vec::new();
    let tag = match payment_op {
        BillOpCode::RequestToPay => "bcr/request-to-pay/v1",
        BillOpCode::OfferToSell => "bcr/offer-to-sell/v1",
        BillOpCode::RequestRecourse => "bcr/request-recourse/v1",
        _ => return Err(Error::Tweak("Invalid operation".to_owned())),
    };
    let hashed_tag = Sha256Hash::from_bytes(tag.as_bytes());
    // tag twice with hashed tag according to bip341
    input.extend_from_slice(&hashed_tag.decode_to_array());
    input.extend_from_slice(&hashed_tag.decode_to_array());
    input.extend_from_slice(&block_id.inner().to_be_bytes());
    input.extend_from_slice(&previous_hash.decode());

    let tweak_hash = Sha256Hash::from_bytes(&input);
    Ok(tweak_hash)
}

#[cfg(test)]
pub mod tests {
    use bitcoin::secp256k1::{SecretKey, global::SECP256K1};
    use bitcoin::{AddressType, Network, PrivateKey};
    use miniscript::{Descriptor, DescriptorPublicKey};

    use super::*;

    fn test_privkeys(network: Network) -> (PrivateKey, PrivateKey) {
        let sk1 = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let sk2 = SecretKey::from_slice(&[2u8; 32]).unwrap();

        (PrivateKey::new(sk1, network), PrivateKey::new(sk2, network))
    }

    fn test_pubkeys(network: Network) -> (PublicKey, PublicKey) {
        let (pk1, pk2) = test_privkeys(network);
        (
            pk1.public_key(SECP256K1).inner,
            pk2.public_key(SECP256K1).inner,
        )
    }

    #[test]
    fn address_is_taproot_on_expected_network() {
        let network = Network::Testnet;
        let (bill_pub, holder_pub) = test_pubkeys(network);

        let tweak_hash =
            Sha256Hash::new("1111111111111111111111111111111111111111111111111111111111111111");

        let addr = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        assert_eq!(
            addr.assume_checked_ref().address_type(),
            Some(AddressType::P2tr)
        );
        assert!(addr.is_valid_for_network(network));
        assert!(addr.assume_checked_ref().to_string().starts_with("tb1p"));
    }

    #[test]
    fn same_keys_and_same_tweak_give_same_address_and_descriptor() {
        let network = Network::Testnet;
        let (bill_pub, holder_pub) = test_pubkeys(network);
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash =
            Sha256Hash::new("1111111111111111111111111111111111111111111111111111111111111111");

        let addr1 = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();
        let addr2 = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let desc1 = get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
            .unwrap();
        let desc2 = get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
            .unwrap();

        assert_eq!(addr1, addr2);
        assert_eq!(desc1, desc2);
    }

    #[test]
    fn different_tweaks_give_different_addresses_and_descriptors() {
        let network = Network::Testnet;
        let (bill_pub, holder_pub) = test_pubkeys(network);
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash_1 =
            Sha256Hash::new("1111111111111111111111111111111111111111111111111111111111111111");
        let tweak_hash_2 =
            Sha256Hash::new("2222222222222222222222222222222222222222222222222222222222222222");

        let addr1 = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash_1, network).unwrap();
        let addr2 = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash_2, network).unwrap();

        let desc1 =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash_1, network)
                .unwrap();
        let desc2 =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash_2, network)
                .unwrap();

        assert_ne!(addr1, addr2);
        assert_ne!(desc1, desc2);
    }

    #[test]
    fn private_descriptor_gives_access_to_same_address() {
        let network = Network::Testnet;
        let (bill_pub, holder_pub) = test_pubkeys(network);
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash =
            Sha256Hash::new("1111111111111111111111111111111111111111111111111111111111111111");

        let address = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let descriptor =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
                .unwrap();

        let (parsed_desc, _kmap) =
            Descriptor::<DescriptorPublicKey>::parse_descriptor(SECP256K1, descriptor.as_str())
                .unwrap();
        let address_from_descriptor = parsed_desc
            .at_derivation_index(0)
            .unwrap()
            .address(network)
            .unwrap();

        assert_eq!(address.assume_checked(), address_from_descriptor);
    }

    #[test]
    fn calculates_tweak_hash_for_request_to_pay() {
        let block_id = BlockId::first().add(1);
        let previous_hash = Sha256Hash::from_bytes(b"previous-hash");

        let result1 = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id,
            &previous_hash,
        )
        .expect("works");
        let result2 = calculate_tweak_hash_for_payment_request(
            BillOpCode::OfferToSell,
            &block_id,
            &previous_hash,
        )
        .expect("works");
        let result3 = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestRecourse,
            &block_id,
            &previous_hash,
        )
        .expect("works");
        assert_ne!(result1, result2);
        assert_ne!(result2, result3);

        let result_invalid =
            calculate_tweak_hash_for_payment_request(BillOpCode::Issue, &block_id, &previous_hash);

        assert!(matches!(result_invalid, Err(Error::Tweak(_))));
    }

    #[test]
    fn hash_depends_on_block_id_and_previous_hash() {
        let previous_hash_1 = Sha256Hash::from_bytes(b"previous-hash-1");
        let previous_hash_2 = Sha256Hash::from_bytes(b"previous-hash-2");
        let block_id_1 = BlockId::first();
        let block_id_2 = BlockId::first().add(2);

        let hash_a = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id_1,
            &previous_hash_1,
        )
        .unwrap();

        let hash_b = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id_2,
            &previous_hash_1,
        )
        .unwrap();

        let hash_c = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id_1,
            &previous_hash_2,
        )
        .unwrap();

        assert_ne!(hash_a, hash_b);
        assert_ne!(hash_a, hash_c);
    }

    #[test]
    fn full_flow_tweak_address_descriptor_validation_all_match() {
        let network = Network::Testnet;
        let block_id = BlockId::first().add(6);
        let previous_hash =
            Sha256Hash::new("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");
        let (bill_pub, holder_pub) = test_pubkeys(network);
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash = calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestRecourse,
            &block_id,
            &previous_hash,
        )
        .unwrap();

        let address = get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let descriptor =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
                .unwrap();

        let (parsed_desc, _kmap) =
            Descriptor::<DescriptorPublicKey>::parse_descriptor(SECP256K1, descriptor.as_str())
                .unwrap();
        let address_from_descriptor = parsed_desc
            .at_derivation_index(0)
            .unwrap()
            .address(network)
            .unwrap();

        assert_eq!(address.clone().assume_checked(), address_from_descriptor);
    }

    #[test]
    fn parse_private_descriptor_matches_created_descriptor_and_address() {
        let network = Network::Testnet;
        let (bill_pub, holder_pub) = test_pubkeys(network);
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash =
            Sha256Hash::new("1111111111111111111111111111111111111111111111111111111111111111");

        let created_descriptor =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
                .unwrap();

        let expected_address =
            get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let (parsed_desc, parsed_privkey, parsed_address) =
            parse_private_descriptor(&created_descriptor, network).unwrap();

        let expected_combined_key = bill_priv
            .inner
            .add_tweak(&Scalar::from(holder_priv.inner))
            .unwrap();

        let tweak = Scalar::from_be_bytes(tweak_hash.decode_to_array()).unwrap();
        let expected_tweaked_key = expected_combined_key.add_tweak(&tweak).unwrap();
        let expected_privkey = PrivateKey::new(expected_tweaked_key, network);

        assert_eq!(parsed_privkey, expected_privkey);
        assert_eq!(parsed_address, expected_address);

        let reparsed_address = parsed_desc
            .derived_descriptor(SECP256K1, 0)
            .unwrap()
            .address(network)
            .unwrap();

        assert_eq!(parsed_address.assume_checked(), reparsed_address);
    }

    #[test]
    fn parse_private_descriptor_roundtrips_to_same_secret_descriptor_string() {
        let network = Network::Testnet;
        let (bill_priv, holder_priv) = test_privkeys(network);

        let tweak_hash =
            Sha256Hash::new("2222222222222222222222222222222222222222222222222222222222222222");

        let created_descriptor =
            get_combined_private_descriptor(&bill_priv, &holder_priv, &tweak_hash, network)
                .unwrap();

        let (parsed_desc, parsed_privkey, _) =
            parse_private_descriptor(&created_descriptor, network).unwrap();

        let single = miniscript::descriptor::SinglePriv {
            key: parsed_privkey,
            origin: None,
        };
        let desc_seckey = miniscript::descriptor::DescriptorSecretKey::Single(single);

        let internal_key = match &parsed_desc {
            miniscript::Descriptor::Tr(tr) => tr.internal_key().clone(),
            _ => panic!("expected taproot descriptor"),
        };

        let kmap =
            miniscript::descriptor::KeyMap::from_iter(std::iter::once((internal_key, desc_seckey)));

        let roundtripped_descriptor =
            BtcDescriptor::new(parsed_desc.to_string_with_secret(&kmap)).unwrap();

        assert_eq!(roundtripped_descriptor, created_descriptor);
    }
}
