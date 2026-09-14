use super::super::bill::BillOpCode;
use super::Result;
use super::{Block, CompanyOpCode};
use crate::protocol::Date;
use crate::protocol::Email;
use crate::protocol::Identification;
use crate::protocol::Name;
use crate::protocol::SchnorrSignature;
use crate::protocol::Sha256Hash;
use crate::protocol::Timestamp;
use crate::protocol::base::identity_proof::{EmailIdentityProofData, SignedIdentityProof};
use crate::protocol::crypto::{self, BcrKeys};
use crate::protocol::{Address, Country, Zip};
use crate::protocol::{BlockId, ProtocolValidationError};
use crate::protocol::{City, EditOptionalFieldMode};
use crate::protocol::{File, PostalAddress};
use bcr_common::core::BillId;
use bcr_common::core::NodeId;
use bitcoin::base58;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use borsh::{from_slice, to_vec};
use borsh_derive::{BorshDeserialize, BorshSerialize};
use log::{error, warn};
use serde::{Deserialize, Serialize};

#[derive(BorshSerialize)]
pub struct CompanyBlockDataToHash {
    company_id: NodeId,
    id: BlockId,
    plaintext_hash: Sha256Hash,
    previous_hash: Sha256Hash,
    data: Vec<u8>, // encrypted block data
    timestamp: Timestamp,
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_pubkey",
        deserialize_with = "crate::protocol::serialization::deserialize_pubkey"
    )]
    public_key: PublicKey,
    signatory_node_id: NodeId,
    op_code: CompanyOpCode,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum SignatoryType {
    Solo,
}

/// Structure for the block data of a company block
///
/// - `data` contains the actual data of the block, encrypted using the company's pub key
/// - `key` is optional and if set, contains the company private keys encrypted by an identity
///   pub key (e.g. for CreateCompany the creator's and InviteSignatory the signatory's)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct CompanyBlockData {
    data: Vec<u8>,
    // The encrypted, base58 encoded SecretKey
    key: Option<String>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyBlock {
    pub company_id: NodeId,
    pub id: BlockId,
    pub plaintext_hash: Sha256Hash,
    pub hash: Sha256Hash,
    pub timestamp: Timestamp,
    pub data: Vec<u8>, // encrypted block data
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_pubkey",
        deserialize_with = "crate::protocol::serialization::deserialize_pubkey"
    )]
    pub public_key: PublicKey,
    pub signatory_node_id: NodeId,
    pub previous_hash: Sha256Hash,
    pub signature: SchnorrSignature,
    pub op_code: CompanyOpCode,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyCreateBlockData {
    pub id: NodeId,
    pub name: Name,
    pub country_of_registration: Option<Country>,
    pub city_of_registration: Option<City>,
    pub postal_address: PostalAddress,
    pub email: Email,
    pub registration_number: Option<Identification>,
    pub registration_date: Option<Date>,
    pub proof_of_registration_file: Option<File>,
    pub logo_file: Option<File>,
    pub creation_time: Timestamp,
    pub creator: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyUpdateBlockData {
    pub name: Option<Name>,
    pub email: Option<Email>,
    pub country: Option<Country>,
    pub city: Option<City>,
    pub zip: EditOptionalFieldMode<Zip>,
    pub address: Option<Address>,
    pub country_of_registration: EditOptionalFieldMode<Country>,
    pub city_of_registration: EditOptionalFieldMode<City>,
    pub registration_number: EditOptionalFieldMode<Identification>,
    pub registration_date: EditOptionalFieldMode<Date>,
    pub logo_file: EditOptionalFieldMode<File>,
    pub proof_of_registration_file: EditOptionalFieldMode<File>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanySignCompanyBillBlockData {
    pub bill_id: BillId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
    pub operation: BillOpCode,
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_optional_privkey",
        deserialize_with = "crate::protocol::serialization::deserialize_optional_privkey"
    )]
    pub bill_key: Option<SecretKey>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyInviteSignatoryBlockData {
    pub invitee: NodeId,
    pub inviter: NodeId,
    pub t: SignatoryType,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanySignatoryAcceptInviteBlockData {
    pub accepter: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanySignatoryRejectInviteBlockData {
    pub rejecter: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyRemoveSignatoryBlockData {
    pub removee: NodeId,
    pub remover: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompanyIdentityProofBlockData {
    pub proof: SignedIdentityProof,
    pub data: EmailIdentityProofData,
    pub reference_block: Option<BlockId>, // the block this identity proof refers to, e.g. an accept, or create block
                                          // optional because for changing email, there's no reference block
}

#[derive(Debug)]
pub struct VerifyAndGetSignerData {
    pub signer: NodeId,
    pub invitee: Option<NodeId>,
    pub removee: Option<NodeId>,
    pub identity_proof_data: Option<(SignedIdentityProof, EmailIdentityProofData, Option<BlockId>)>,
}

#[derive(Debug)]
pub enum CompanyBlockPayload {
    Create(CompanyCreateBlockData),
    Update(CompanyUpdateBlockData),
    SignBill(CompanySignCompanyBillBlockData),
    InviteSignatory(CompanyInviteSignatoryBlockData),
    SignatoryAcceptInvite(CompanySignatoryAcceptInviteBlockData),
    SignatoryRejectInvite(CompanySignatoryRejectInviteBlockData),
    RemoveSignatory(CompanyRemoveSignatoryBlockData),
    IdentityProof(CompanyIdentityProofBlockData),
}

impl Block for CompanyBlock {
    type OpCode = CompanyOpCode;
    type BlockDataToHash = CompanyBlockDataToHash;

    fn id(&self) -> BlockId {
        self.id
    }

    fn timestamp(&self) -> Timestamp {
        self.timestamp
    }

    fn op_code(&self) -> &Self::OpCode {
        &self.op_code
    }

    fn plaintext_hash(&self) -> &Sha256Hash {
        &self.plaintext_hash
    }

    fn hash(&self) -> &Sha256Hash {
        &self.hash
    }

    fn previous_hash(&self) -> &Sha256Hash {
        &self.previous_hash
    }

    fn data(&self) -> &[u8] {
        &self.data
    }

    fn signature(&self) -> &SchnorrSignature {
        &self.signature
    }

    fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    fn validate(&self) -> bool {
        true
    }

    /// We validate the plaintext hash against the plaintext data from the CompanyBlockData wrapper
    fn validate_plaintext_hash(&self, private_key: &bitcoin::secp256k1::SecretKey) -> bool {
        match from_slice::<CompanyBlockData>(self.data()) {
            Ok(data_wrapper) => match crypto::decrypt_ecies(&data_wrapper.data, private_key) {
                Ok(decrypted) => self.plaintext_hash() == &Sha256Hash::from_bytes(&decrypted),
                Err(e) => {
                    error!(
                        "Decrypt Error while validating plaintext hash for id {}: {e}",
                        self.id()
                    );
                    false
                }
            },
            Err(e) => {
                error!(
                    "Wrapper Deserialize Error while validating plaintext hash for id {}: {e}",
                    self.id()
                );
                false
            }
        }
    }

    fn get_block_data_to_hash(&self) -> Self::BlockDataToHash {
        CompanyBlockDataToHash {
            company_id: self.company_id.clone(),
            id: self.id(),
            plaintext_hash: self.plaintext_hash().to_owned(),
            previous_hash: self.previous_hash().to_owned(),
            data: self.data().to_owned(),
            timestamp: self.timestamp(),
            public_key: self.public_key().to_owned(),
            signatory_node_id: self.signatory_node_id.clone(),
            op_code: self.op_code().to_owned(),
        }
    }
}

impl CompanyBlock {
    /// Create a new block and sign it with an aggregated key, combining the identity key of the
    /// signer and the company key
    fn new(
        company_id: NodeId,
        id: BlockId,
        previous_hash: Sha256Hash,
        data: Vec<u8>,
        op_code: CompanyOpCode,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
        plaintext_hash: Sha256Hash,
    ) -> Result<Self> {
        // The order here is important: identity -> company
        let keys: Vec<bitcoin::secp256k1::SecretKey> = vec![
            identity_keys.get_private_key(),
            company_keys.get_private_key(),
        ];
        let signatory_node_id = NodeId::new(identity_keys.pub_key(), company_id.network());
        let aggregated_public_key = SchnorrSignature::get_aggregated_public_key(&keys)?;
        let hash = Self::calculate_hash(CompanyBlockDataToHash {
            company_id: company_id.clone(),
            id,
            plaintext_hash: plaintext_hash.clone(),
            previous_hash: previous_hash.clone(),
            data: data.clone(),
            timestamp,
            public_key: aggregated_public_key.to_owned(),
            signatory_node_id: signatory_node_id.clone(),
            op_code: op_code.clone(),
        })?;
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys)?;

        Ok(Self {
            company_id,
            id,
            plaintext_hash,
            hash,
            timestamp,
            previous_hash,
            signature,
            public_key: aggregated_public_key,
            signatory_node_id,
            data,
            op_code,
        })
    }

    pub fn create_block_for_create(
        company_id: NodeId,
        genesis_hash: Sha256Hash,
        company: &CompanyCreateBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        if identity_keys.pub_key() != company.creator.pub_key() {
            return Err(super::super::Error::BlockDataInvalid(
                ProtocolValidationError::CompanySignerCreatorMismatch,
            ));
        }

        let company_bytes = to_vec(company)?;
        let plaintext_hash = Self::calculate_plaintext_hash(company)?;
        // encrypt data using company pub key
        let encrypted_data = crypto::encrypt_ecies(&company_bytes, &company_keys.pub_key())?;

        let key_bytes = to_vec(&company_keys.get_private_key_string())?;
        // encrypt company keys using creator's identity pub key
        let encrypted_key = base58::encode(&crypto::encrypt_ecies(
            &key_bytes,
            &identity_keys.pub_key(),
        )?);

        let data = CompanyBlockData {
            data: encrypted_data,
            key: Some(encrypted_key),
        };
        let serialized_data = to_vec(&data)?;

        Self::new(
            company_id.to_owned(),
            BlockId::first(),
            genesis_hash,
            serialized_data,
            CompanyOpCode::Create,
            identity_keys,
            company_keys,
            timestamp,
            plaintext_hash,
        )
    }

    pub fn create_block_for_update(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanyUpdateBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::Update,
        )?;
        Ok(block)
    }

    pub fn create_block_for_sign_company_bill(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanySignCompanyBillBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::SignCompanyBill,
        )?;
        Ok(block)
    }

    pub fn create_block_for_invite_signatory(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanyInviteSignatoryBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        signatory_public_key: &PublicKey, // the signatory's public key
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            Some(signatory_public_key),
            timestamp,
            CompanyOpCode::InviteSignatory,
        )?;
        Ok(block)
    }

    pub fn create_block_for_accept_signatory_invite(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanySignatoryAcceptInviteBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::SignatoryAcceptInvite,
        )?;
        Ok(block)
    }

    pub fn create_block_for_reject_signatory_invite(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanySignatoryRejectInviteBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::SignatoryRejectInvite,
        )?;
        Ok(block)
    }

    pub fn create_block_for_remove_signatory(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanyRemoveSignatoryBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::RemoveSignatory,
        )?;
        Ok(block)
    }

    pub fn create_block_for_identity_proof(
        company_id: NodeId,
        previous_block: &Self,
        data: &CompanyIdentityProofBlockData,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            company_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            None,
            timestamp,
            CompanyOpCode::IdentityProof,
        )?;
        Ok(block)
    }

    pub fn get_block_data(&self, company_keys: &BcrKeys) -> Result<CompanyBlockPayload> {
        let data = self.get_decrypted_block_bytes(company_keys)?;
        let result: CompanyBlockPayload = match self.op_code {
            CompanyOpCode::Create => CompanyBlockPayload::Create(from_slice(&data)?),
            CompanyOpCode::Update => CompanyBlockPayload::Update(from_slice(&data)?),
            CompanyOpCode::InviteSignatory => {
                CompanyBlockPayload::InviteSignatory(from_slice(&data)?)
            }
            CompanyOpCode::SignatoryAcceptInvite => {
                CompanyBlockPayload::SignatoryAcceptInvite(from_slice(&data)?)
            }
            CompanyOpCode::SignatoryRejectInvite => {
                CompanyBlockPayload::SignatoryRejectInvite(from_slice(&data)?)
            }
            CompanyOpCode::RemoveSignatory => {
                CompanyBlockPayload::RemoveSignatory(from_slice(&data)?)
            }
            CompanyOpCode::SignCompanyBill => CompanyBlockPayload::SignBill(from_slice(&data)?),
            CompanyOpCode::IdentityProof => CompanyBlockPayload::IdentityProof(from_slice(&data)?),
        };
        Ok(result)
    }

    pub(super) fn get_decrypted_block_bytes(&self, company_keys: &BcrKeys) -> Result<Vec<u8>> {
        let block_data: CompanyBlockData = from_slice(&self.data)?;
        let decrypted_bytes =
            crypto::decrypt_ecies(&block_data.data, &company_keys.get_private_key())?;
        Ok(decrypted_bytes)
    }

    fn encrypt_data_create_block_and_validate<T: borsh::BorshSerialize>(
        company_id: NodeId,
        previous_block: &Self,
        data: &T,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        public_key_for_keys: Option<&PublicKey>,
        timestamp: Timestamp,
        op_code: CompanyOpCode,
    ) -> Result<Self> {
        let bytes = to_vec(&data)?;
        let plaintext_hash = Self::calculate_plaintext_hash(data)?;
        // encrypt data using the company pub key
        let encrypted_data = crypto::encrypt_ecies(&bytes, &company_keys.pub_key())?;

        let mut key = None;

        // in case there are keys to encrypt, encrypt them using the receiver's identity pub key
        if op_code == CompanyOpCode::InviteSignatory
            && let Some(signatory_public_key) = public_key_for_keys
        {
            let key_bytes = to_vec(&company_keys.get_private_key_string())?;
            let encrypted_key =
                base58::encode(&crypto::encrypt_ecies(&key_bytes, signatory_public_key)?);
            key = Some(encrypted_key);
        }

        let data = CompanyBlockData {
            data: encrypted_data,
            key,
        };
        let serialized_data = to_vec(&data)?;

        let new_block = Self::new(
            company_id,
            BlockId::next_from_previous_block_id(&previous_block.id),
            previous_block.hash.clone(),
            serialized_data,
            op_code,
            identity_keys,
            company_keys,
            timestamp,
            plaintext_hash,
        )?;

        if !new_block.validate_with_previous(previous_block) {
            return Err(super::super::Error::BlockInvalid);
        }
        Ok(new_block)
    }

    pub fn verify_and_get_signer(&self, company_keys: &BcrKeys) -> Result<VerifyAndGetSignerData> {
        let company_id = self.company_id.clone();

        let (signer, invitee, removee, identity_proof_data) = match self
            .get_block_data(company_keys)?
        {
            CompanyBlockPayload::Create(data) => (data.creator, None, None, None),
            CompanyBlockPayload::Update(_) => (self.signatory_node_id.clone(), None, None, None),
            CompanyBlockPayload::SignBill(_) => (self.signatory_node_id.clone(), None, None, None),
            CompanyBlockPayload::InviteSignatory(data) => {
                (data.inviter, Some(data.invitee), None, None)
            }
            CompanyBlockPayload::SignatoryAcceptInvite(data) => (data.accepter, None, None, None),
            CompanyBlockPayload::SignatoryRejectInvite(data) => (data.rejecter, None, None, None),
            CompanyBlockPayload::RemoveSignatory(data) => {
                (data.remover, None, Some(data.removee), None)
            }
            CompanyBlockPayload::IdentityProof(data) => (
                data.data.node_id.clone(),
                None,
                None,
                Some((data.proof, data.data, data.reference_block)),
            ),
        };

        if !self.verify_signer(&signer, company_keys)? {
            return Err(super::super::Error::BlockSignatureDoesNotMatchSigner);
        }

        // If the identity proof doesn't match with the signer, or isn't valid, we show a warning
        if let Some(ref ip) = identity_proof_data {
            let (proof, data) = (&ip.0, &ip.1);
            if data.node_id != signer || data.company_node_id != Some(company_id.clone()) {
                warn!(
                    "Identity Proof Verification failed for company {}, block {} and signer {}, signatory {} - signatory and signer don't match with identity proof",
                    self.company_id,
                    self.id(),
                    company_id,
                    signer
                );
            }

            if !proof.verify(data).unwrap_or(false) {
                warn!(
                    "Identity Proof Verification failed for company {}, block {} and signer {}, signatory {}",
                    self.company_id,
                    self.id(),
                    company_id,
                    signer
                );
            }
        }

        Ok(VerifyAndGetSignerData {
            signer,
            invitee,
            removee,
            identity_proof_data,
        })
    }

    fn verify_signer(&self, signatory: &NodeId, company_keys: &BcrKeys) -> Result<bool> {
        let keys: Vec<PublicKey> = vec![
            // add company signatory first, since it's the identity key
            signatory.pub_key(),
            // then, add the company key
            company_keys.pub_key(),
        ];
        let aggregated_public_key = match SchnorrSignature::combine_pub_keys(&keys) {
            Ok(res) => res,
            Err(e) => {
                error!(
                    "Error while aggregating keys for block id {}: {e}",
                    self.id()
                );
                return Ok(false);
            }
        };
        match self.signature().verify(self.hash(), &aggregated_public_key) {
            Err(e) => {
                error!("Error while verifying block id {}: {e}", self.id());
                Ok(false)
            }
            Ok(res) => Ok(res),
        }
    }
}
