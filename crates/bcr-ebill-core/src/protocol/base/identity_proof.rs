use bitcoin::secp256k1::{SECP256K1, SecretKey};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use bcr_common::core::NodeId;

use crate::protocol::{
    Email, ProtocolError, SchnorrSignature, Sha256Hash, Timestamp, crypto::Error as CryptoError,
};

/// The signature and witness of an identity proof
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedIdentityProof {
    /// The signature of the sha256 hashed borsh-payload (EmailIdentityProofData) by the mint
    pub signature: SchnorrSignature,
    /// The mint (signer) node id
    pub witness: NodeId,
}

impl SignedIdentityProof {
    pub fn verify(&self, data: &EmailIdentityProofData) -> Result<bool, ProtocolError> {
        let serialized = borsh::to_vec(&data)?;
        let hash = Sha256Hash::from_bytes(&serialized);
        let res = self.signature.verify(&hash, &self.witness.pub_key())?;
        Ok(res)
    }
}

/// Mapping from (node_id/option<company_node_id>) => email, to be signed by a witness (mint)
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailIdentityProofData {
    /// Identity node id
    pub node_id: NodeId,
    /// Optional company node id
    pub company_node_id: Option<NodeId>,
    /// The mapped email
    pub email: Email,
    /// The time of signing
    pub created_at: Timestamp,
}

impl EmailIdentityProofData {
    pub fn sign(
        &self,
        witness: &NodeId,
        witness_private_key: &SecretKey,
    ) -> Result<SignedIdentityProof, ProtocolError> {
        if witness.pub_key() != witness_private_key.public_key(SECP256K1) {
            return Err(ProtocolError::Crypto(CryptoError::Crypto(
                "Keys don't match".into(),
            )));
        }
        let serialized = borsh::to_vec(&self)?;
        let hash = Sha256Hash::from_bytes(&serialized);
        let sig = SchnorrSignature::sign(&hash, witness_private_key)?;
        Ok(SignedIdentityProof {
            signature: sig,
            witness: witness.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::tests::tests::{node_id_test, private_key_test, test_ts};

    use super::*;

    #[test]
    fn test_sign_verify() {
        let data = EmailIdentityProofData {
            node_id: node_id_test(),
            company_node_id: None,
            email: Email::new("test@example.com").unwrap(),
            created_at: test_ts(),
        };
        let proof = data
            .sign(&node_id_test(), &private_key_test())
            .expect("works");
        assert!(proof.verify(&data).expect("works"));
    }
}
