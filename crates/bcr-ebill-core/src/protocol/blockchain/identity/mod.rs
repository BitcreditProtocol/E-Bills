use super::Result;
use super::bill::BillOpCode;
use super::{Block, Blockchain};
use crate::protocol::Country;
use crate::protocol::Date;
use crate::protocol::Email;
use crate::protocol::Identification;
use crate::protocol::Name;
use crate::protocol::SchnorrSignature;
use crate::protocol::Sha256Hash;
use crate::protocol::Timestamp;
use crate::protocol::base::identity_proof::{EmailIdentityProofData, SignedIdentityProof};
use crate::protocol::blockchain::{Error, borsh_to_json_value};
use crate::protocol::crypto::{self, BcrKeys};
use crate::protocol::{Address, City, Zip};
use crate::protocol::{BlockId, EditOptionalFieldMode};
use crate::protocol::{Field, ProtocolValidationError, Validate};
use crate::protocol::{File, OptionalPostalAddress};
use bcr_common::core::{BillId, NodeId};
use bitcoin::secp256k1::{PublicKey, SecretKey};
use borsh::{from_slice, to_vec};
use borsh_derive::{BorshDeserialize, BorshSerialize};
use log::{error, warn};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

pub mod validation;

#[derive(
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
    Debug,
    Clone,
    PartialEq,
    EnumString,
    Display,
)]
pub enum IdentityOpCode {
    Create,
    Update,
    SignPersonBill,
    SignCompanyBill,
    CreateCompany,
    InviteSignatory,
    AcceptSignatoryInvite,
    RejectSignatoryInvite,
    RemoveSignatory,
    IdentityProof,
}

#[derive(Debug, Clone)]
pub struct IdentityValidateActionData {
    pub blockchain: IdentityBlockchain,
    pub id: NodeId,
    pub op: IdentityOpCode,
    pub keys: BcrKeys,
    pub identity_proof_data: Option<(SignedIdentityProof, EmailIdentityProofData)>,
}

#[derive(BorshSerialize)]
pub struct IdentityBlockDataToHash {
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
    op_code: IdentityOpCode,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityBlock {
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
    pub previous_hash: Sha256Hash,
    pub signature: SchnorrSignature,
    pub op_code: IdentityOpCode,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone)]
pub struct IdentityBlockPlaintextWrapper {
    pub block: IdentityBlock,
    pub plaintext_data_bytes: Vec<u8>,
}

impl IdentityBlockPlaintextWrapper {
    /// This is only used for dev mode
    pub fn to_json_text(&self) -> Result<String> {
        let mut serialized =
            serde_json::to_value(&self.block).map_err(|e| Error::JSON(e.to_string()))?;

        let block_data: serde_json::Value = match self.block.op_code() {
            IdentityOpCode::Create => {
                borsh_to_json_value::<IdentityCreateBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::Update => {
                borsh_to_json_value::<IdentityUpdateBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::InviteSignatory => {
                borsh_to_json_value::<IdentityInviteSignatoryBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::AcceptSignatoryInvite => borsh_to_json_value::<
                IdentityAcceptSignatoryInviteBlockData,
            >(&self.plaintext_data_bytes)?,
            IdentityOpCode::RejectSignatoryInvite => borsh_to_json_value::<
                IdentityRejectSignatoryInviteBlockData,
            >(&self.plaintext_data_bytes)?,
            IdentityOpCode::RemoveSignatory => {
                borsh_to_json_value::<IdentityRemoveSignatoryBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::SignPersonBill => {
                borsh_to_json_value::<IdentitySignPersonBillBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::SignCompanyBill => {
                borsh_to_json_value::<IdentitySignCompanyBillBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::CreateCompany => {
                borsh_to_json_value::<IdentityCreateCompanyBlockData>(&self.plaintext_data_bytes)?
            }
            IdentityOpCode::IdentityProof => {
                borsh_to_json_value::<IdentityProofBlockData>(&self.plaintext_data_bytes)?
            }
        };

        if let Some(obj) = serialized.as_object_mut() {
            obj.insert("data".to_string(), block_data);
        } else {
            return Err(Error::JSON(
                "Block didn't serialize to JSON object".to_string(),
            ));
        }

        serde_json::to_string(&serialized).map_err(|e| Error::JSON(e.to_string()))
    }
}

#[derive(Debug)]
pub struct VerifyAndGetSignerData {
    pub signer: Option<NodeId>,
    pub identity_proof_data: Option<(SignedIdentityProof, EmailIdentityProofData)>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityCreateBlockData {
    pub t: IdentityType,
    pub node_id: NodeId,
    pub name: Name,
    pub email: Option<Email>,
    pub postal_address: OptionalPostalAddress,
    pub date_of_birth: Option<Date>,
    pub city_of_birth: Option<City>,
    pub country_of_birth: Option<Country>,
    pub identification_number: Option<Identification>,
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_vec_url",
        deserialize_with = "crate::protocol::serialization::deserialize_vec_url"
    )]
    pub nostr_relays: Vec<url::Url>,
    pub profile_picture_file: Option<File>,
    pub identity_document_file: Option<File>,
}

impl Validate for IdentityCreateBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if let IdentityType::Ident = self.t {
            // email needs to be set and not blank
            if self.email.is_none() {
                return Err(ProtocolValidationError::FieldEmpty(Field::Email));
            }
            // For Ident, the postal address needs to be fully set
            self.postal_address.validate_to_be_non_optional()?;
        }
        Ok(())
    }
}

#[repr(u8)]
#[derive(
    Debug,
    Clone,
    serde_repr::Serialize_repr,
    serde_repr::Deserialize_repr,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
)]
#[borsh(use_discriminant = true)]
pub enum IdentityType {
    Ident = 0,
    Anon = 1,
}

impl TryFrom<u64> for IdentityType {
    type Error = ProtocolValidationError;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(IdentityType::Ident),
            1 => Ok(IdentityType::Anon),
            _ => Err(ProtocolValidationError::InvalidIdentityType),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityUpdateBlockData {
    pub t: Option<IdentityType>, // for deanonymization
    pub name: Option<Name>,
    pub email: Option<Email>,
    pub country: Option<Country>,
    pub city: Option<City>,
    pub zip: EditOptionalFieldMode<Zip>,
    pub address: Option<Address>,
    pub date_of_birth: EditOptionalFieldMode<Date>,
    pub country_of_birth: EditOptionalFieldMode<Country>,
    pub city_of_birth: EditOptionalFieldMode<City>,
    pub identification_number: EditOptionalFieldMode<Identification>,
    pub profile_picture_file: EditOptionalFieldMode<File>,
    pub identity_document_file: EditOptionalFieldMode<File>,
}

impl Validate for IdentityUpdateBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        // deanonymization
        if let Some(IdentityType::Ident) = self.t {
            // email needs to be set and not blank
            if self.email.is_none() {
                return Err(ProtocolValidationError::FieldEmpty(Field::Email));
            }
            // For Ident, the postal address needs to be fully set
            let addr = OptionalPostalAddress {
                country: self.country.clone(),
                city: self.city.clone(),
                zip: None, // irrelevant for the validation
                address: self.address.clone(),
            };
            addr.validate_to_be_non_optional()?;
        }
        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentitySignPersonBillBlockData {
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
pub struct IdentitySignCompanyBillBlockData {
    pub bill_id: BillId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
    pub company_id: NodeId,
    pub operation: BillOpCode,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityCreateCompanyBlockData {
    pub company_id: NodeId,
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_privkey",
        deserialize_with = "crate::protocol::serialization::deserialize_privkey"
    )]
    pub company_key: SecretKey,
    pub block_hash: Sha256Hash,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityInviteSignatoryBlockData {
    pub company_id: NodeId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
    pub signatory: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityAcceptSignatoryInviteBlockData {
    pub company_id: NodeId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityRejectSignatoryInviteBlockData {
    pub company_id: NodeId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityRemoveSignatoryBlockData {
    pub company_id: NodeId,
    pub block_id: BlockId,
    pub block_hash: Sha256Hash,
    pub signatory: NodeId,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct IdentityProofBlockData {
    pub proof: SignedIdentityProof,
    pub data: EmailIdentityProofData,
}

#[derive(Debug)]
pub enum IdentityBlockPayload {
    Create(IdentityCreateBlockData),
    Update(IdentityUpdateBlockData),
    SignPersonalBill(IdentitySignPersonBillBlockData),
    SignCompanyBill(IdentitySignCompanyBillBlockData),
    CreateCompany(IdentityCreateCompanyBlockData),
    InviteSignatory(IdentityInviteSignatoryBlockData),
    AcceptSignatoryInvite(IdentityAcceptSignatoryInviteBlockData),
    RejectSignatoryInvite(IdentityRejectSignatoryInviteBlockData),
    RemoveSignatory(IdentityRemoveSignatoryBlockData),
    IdentityProof(IdentityProofBlockData),
}

impl Block for IdentityBlock {
    type OpCode = IdentityOpCode;
    type BlockDataToHash = IdentityBlockDataToHash;

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

    fn validate_plaintext_hash(&self, private_key: &bitcoin::secp256k1::SecretKey) -> bool {
        match crypto::decrypt_ecies(self.data(), private_key) {
            Ok(decrypted) => self.plaintext_hash() == &Sha256Hash::from_bytes(&decrypted),
            Err(e) => {
                error!(
                    "Decrypt Error while validating plaintext hash for id {}: {e}",
                    self.id()
                );
                false
            }
        }
    }

    fn get_block_data_to_hash(&self) -> Self::BlockDataToHash {
        IdentityBlockDataToHash {
            id: self.id(),
            plaintext_hash: self.plaintext_hash().to_owned(),
            previous_hash: self.previous_hash().to_owned(),
            data: self.data().to_owned(),
            timestamp: self.timestamp(),
            public_key: self.public_key().to_owned(),
            op_code: self.op_code().to_owned(),
        }
    }
}

impl IdentityBlock {
    fn new(
        id: BlockId,
        previous_hash: Sha256Hash,
        data: Vec<u8>,
        op_code: IdentityOpCode,
        keys: &BcrKeys,
        timestamp: Timestamp,
        plaintext_hash: Sha256Hash,
    ) -> Result<Self> {
        let hash = Self::calculate_hash(IdentityBlockDataToHash {
            id,
            plaintext_hash: plaintext_hash.clone(),
            previous_hash: previous_hash.clone(),
            data: data.clone(),
            timestamp,
            public_key: keys.pub_key(),
            op_code: op_code.clone(),
        })?;
        let signature = SchnorrSignature::sign(&hash, &keys.get_private_key())?;

        Ok(Self {
            id,
            plaintext_hash,
            hash,
            timestamp,
            previous_hash,
            signature,
            public_key: keys.pub_key(),
            data,
            op_code,
        })
    }

    pub fn create_block_for_create(
        genesis_hash: Sha256Hash,
        identity: &IdentityCreateBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let identity_bytes = to_vec(identity)?;
        let plaintext_hash = Self::calculate_plaintext_hash(identity)?;

        let encrypted_data = crypto::encrypt_ecies(&identity_bytes, &keys.pub_key())?;

        Self::new(
            BlockId::first(),
            genesis_hash,
            encrypted_data,
            IdentityOpCode::Create,
            keys,
            timestamp,
            plaintext_hash,
        )
    }

    pub fn create_block_for_update(
        previous_block: &Self,
        data: &IdentityUpdateBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::Update,
        )?;
        Ok(block)
    }

    pub fn create_block_for_sign_person_bill(
        previous_block: &Self,
        data: &IdentitySignPersonBillBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::SignPersonBill,
        )?;
        Ok(block)
    }

    pub fn create_block_for_sign_company_bill(
        previous_block: &Self,
        data: &IdentitySignCompanyBillBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::SignCompanyBill,
        )?;
        Ok(block)
    }

    pub fn create_block_for_create_company(
        previous_block: &Self,
        data: &IdentityCreateCompanyBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::CreateCompany,
        )?;
        Ok(block)
    }

    pub fn create_block_for_invite_signatory(
        previous_block: &Self,
        data: &IdentityInviteSignatoryBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::InviteSignatory,
        )?;
        Ok(block)
    }

    pub fn create_block_for_accept_signatory_invite(
        previous_block: &Self,
        data: &IdentityAcceptSignatoryInviteBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::AcceptSignatoryInvite,
        )?;
        Ok(block)
    }

    pub fn create_block_for_reject_signatory_invite(
        previous_block: &Self,
        data: &IdentityRejectSignatoryInviteBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::RejectSignatoryInvite,
        )?;
        Ok(block)
    }

    pub fn create_block_for_remove_signatory(
        previous_block: &Self,
        data: &IdentityRemoveSignatoryBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::RemoveSignatory,
        )?;
        Ok(block)
    }

    pub fn create_block_for_identity_proof(
        previous_block: &Self,
        data: &IdentityProofBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            previous_block,
            data,
            keys,
            timestamp,
            IdentityOpCode::IdentityProof,
        )?;
        Ok(block)
    }

    fn encrypt_data_create_block_and_validate<T: borsh::BorshSerialize>(
        previous_block: &Self,
        data: &T,
        keys: &BcrKeys,
        timestamp: Timestamp,
        op_code: IdentityOpCode,
    ) -> Result<Self> {
        let bytes = to_vec(&data)?;
        let plaintext_hash = Self::calculate_plaintext_hash(data)?;

        let encrypted_data = crypto::encrypt_ecies(&bytes, &keys.pub_key())?;

        let new_block = Self::new(
            BlockId::next_from_previous_block_id(&previous_block.id()),
            previous_block.hash.clone(),
            encrypted_data,
            op_code,
            keys,
            timestamp,
            plaintext_hash,
        )?;

        if !new_block.validate_with_previous(previous_block) {
            return Err(super::Error::BlockInvalid);
        }
        Ok(new_block)
    }

    pub fn get_block_data(&self, keys: &BcrKeys) -> Result<IdentityBlockPayload> {
        let data = self.get_decrypted_block_bytes(keys)?;
        let result: IdentityBlockPayload = match self.op_code {
            IdentityOpCode::Create => IdentityBlockPayload::Create(from_slice(&data)?),
            IdentityOpCode::Update => IdentityBlockPayload::Update(from_slice(&data)?),
            IdentityOpCode::SignPersonBill => {
                IdentityBlockPayload::SignPersonalBill(from_slice(&data)?)
            }
            IdentityOpCode::SignCompanyBill => {
                IdentityBlockPayload::SignCompanyBill(from_slice(&data)?)
            }
            IdentityOpCode::CreateCompany => {
                IdentityBlockPayload::CreateCompany(from_slice(&data)?)
            }
            IdentityOpCode::InviteSignatory => {
                IdentityBlockPayload::InviteSignatory(from_slice(&data)?)
            }
            IdentityOpCode::AcceptSignatoryInvite => {
                IdentityBlockPayload::AcceptSignatoryInvite(from_slice(&data)?)
            }
            IdentityOpCode::RejectSignatoryInvite => {
                IdentityBlockPayload::RejectSignatoryInvite(from_slice(&data)?)
            }
            IdentityOpCode::RemoveSignatory => {
                IdentityBlockPayload::RemoveSignatory(from_slice(&data)?)
            }
            IdentityOpCode::IdentityProof => {
                IdentityBlockPayload::IdentityProof(from_slice(&data)?)
            }
        };
        Ok(result)
    }

    fn get_decrypted_block_bytes(&self, keys: &BcrKeys) -> Result<Vec<u8>> {
        let decrypted_bytes = crypto::decrypt_ecies(&self.data, &keys.get_private_key())?;
        Ok(decrypted_bytes)
    }

    pub fn verify_and_get_signer(&self, keys: &BcrKeys) -> Result<VerifyAndGetSignerData> {
        let (signer, identity_proof_data) = match self.get_block_data(keys)? {
            IdentityBlockPayload::Create(data) => (Some(data.node_id), None),
            IdentityBlockPayload::IdentityProof(data) => (
                Some(data.data.node_id.clone()),
                Some((data.proof, data.data)),
            ),
            _ => (None, None),
        };

        if !self.verify_sig(keys)? {
            return Err(super::Error::BlockSignatureDoesNotMatchSigner);
        }

        // If the identity proof doesn't match with the signer, or isn't valid, we show a warning
        if let Some(ref ip) = identity_proof_data
            && let Some(ref sig) = signer
        {
            let (proof, data) = (&ip.0, &ip.1);
            if data.node_id != *sig {
                warn!(
                    "Identity Proof Verification failed for identity block {} and signer {}",
                    self.id(),
                    sig
                );
            }

            if !proof.verify(data).unwrap_or(false) {
                warn!(
                    "Identity Proof Verification failed for identity block {} and signer {}",
                    self.id(),
                    sig
                );
            }
        }

        Ok(VerifyAndGetSignerData {
            signer,
            identity_proof_data,
        })
    }

    fn verify_sig(&self, keys: &BcrKeys) -> Result<bool> {
        match self.signature().verify(self.hash(), &keys.pub_key()) {
            Err(e) => {
                error!("Error while verifying block id {}: {e}", self.id());
                Ok(false)
            }
            Ok(res) => Ok(res),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IdentityBlockchain {
    blocks: Vec<IdentityBlock>,
}

impl Blockchain for IdentityBlockchain {
    type Block = IdentityBlock;

    fn blocks(&self) -> &Vec<Self::Block> {
        &self.blocks
    }

    fn blocks_mut(&mut self) -> &mut Vec<Self::Block> {
        &mut self.blocks
    }
}

impl IdentityBlockchain {
    /// Creates a new identity chain
    pub fn new(
        identity: &IdentityCreateBlockData,
        keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let genesis_hash = Sha256Hash::from_bytes(keys.get_public_key().as_bytes());

        let first_block =
            IdentityBlock::create_block_for_create(genesis_hash, identity, keys, timestamp)?;

        Ok(Self {
            blocks: vec![first_block],
        })
    }

    pub fn is_creator(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<bool> {
        if let IdentityBlockPayload::Create(block_data) =
            self.get_first_block().get_block_data(keys)?
            && &block_data.node_id == node_id
        {
            return Ok(true);
        }
        Ok(false)
    }

    // creator and at least one identity proof block
    pub fn is_identified(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<bool> {
        if self.is_creator(node_id, keys)? {
            for block in self.blocks().iter() {
                if matches!(block.op_code(), IdentityOpCode::IdentityProof) {
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            Ok(false)
        }
    }

    /// Creates an identity chain from a list of blocks
    pub fn new_from_blocks(blocks_to_add: Vec<IdentityBlock>) -> Result<Self> {
        match blocks_to_add.first() {
            None => Err(super::Error::BlockchainInvalid),
            Some(first) => {
                if !first.verify() || !first.validate_hash() {
                    return Err(super::Error::BlockchainInvalid);
                }

                let chain = Self {
                    blocks: blocks_to_add,
                };

                if !chain.is_chain_valid() {
                    return Err(super::Error::BlockchainInvalid);
                }

                Ok(chain)
            }
        }
    }

    pub fn get_chain_with_plaintext_block_data(
        &self,
        keys: &BcrKeys,
    ) -> Result<Vec<IdentityBlockPlaintextWrapper>> {
        let mut result = Vec::with_capacity(self.blocks().len());
        for block in self.blocks.iter() {
            let plaintext_data_bytes = match block.op_code() {
                IdentityOpCode::Create => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::Update => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::InviteSignatory => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::AcceptSignatoryInvite => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::RejectSignatoryInvite => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::RemoveSignatory => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::SignCompanyBill => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::IdentityProof => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::SignPersonBill => block.get_decrypted_block_bytes(keys)?,
                IdentityOpCode::CreateCompany => block.get_decrypted_block_bytes(keys)?,
            };

            if block.plaintext_hash != Sha256Hash::from_bytes(&plaintext_data_bytes) {
                return Err(Error::BlockInvalid);
            }

            result.push(IdentityBlockPlaintextWrapper {
                block: block.clone(),
                plaintext_data_bytes,
            });
        }

        // Validate the chain from the wrapper
        IdentityBlockchain::new_from_blocks(
            result
                .iter()
                .map(|wrapper| wrapper.block.to_owned())
                .collect::<Vec<IdentityBlock>>(),
        )?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::tests::{
        bill_id_test, empty_identity, node_id_test, private_key_test, signed_identity_proof_test,
        test_ts,
    };

    #[test]
    fn test_plaintext_hash() {
        let identity = empty_identity();
        let keys = BcrKeys::new();

        let chain = IdentityBlockchain::new(&identity, &keys, test_ts());
        assert!(chain.is_ok());
        assert!(chain.as_ref().unwrap().is_chain_valid());
        assert!(
            chain.as_ref().unwrap().blocks()[0].validate_plaintext_hash(&keys.get_private_key())
        );
    }

    #[test]
    fn create_and_check_validity() {
        let identity = empty_identity();

        let chain = IdentityBlockchain::new(&identity, &BcrKeys::new(), test_ts());
        assert!(chain.is_ok());
        assert!(chain.as_ref().unwrap().is_chain_valid());
    }

    #[test]
    fn multi_block() {
        let identity = empty_identity();
        let keys = BcrKeys::new();

        let chain = IdentityBlockchain::new(&identity, &keys, test_ts());
        assert!(chain.is_ok());
        assert!(chain.as_ref().unwrap().is_chain_valid());
        let mut chain = chain.unwrap();

        let update_block = IdentityBlock::create_block_for_update(
            chain.get_latest_block(),
            &IdentityUpdateBlockData {
                t: None,
                name: Some(Name::new("newname").unwrap()),
                email: None,
                country: Some(Country::AT),
                city: Some(City::new("Vienna").unwrap()),
                zip: EditOptionalFieldMode::Set(Zip::new("1010").unwrap()),
                address: Some(Address::new("Kärntner Straße 1").unwrap()),
                date_of_birth: EditOptionalFieldMode::Ignore,
                country_of_birth: EditOptionalFieldMode::Ignore,
                city_of_birth: EditOptionalFieldMode::Ignore,
                identification_number: EditOptionalFieldMode::Ignore,
                profile_picture_file: EditOptionalFieldMode::Ignore,
                identity_document_file: EditOptionalFieldMode::Ignore,
            },
            &keys,
            test_ts(),
        );
        assert!(update_block.is_ok());
        chain.try_add_block(update_block.unwrap());

        let sign_person_bill_block = IdentityBlock::create_block_for_sign_person_bill(
            chain.get_latest_block(),
            &IdentitySignPersonBillBlockData {
                bill_id: bill_id_test(),
                block_id: BlockId::first(),
                block_hash: Sha256Hash::new("some hash"),
                operation: BillOpCode::Issue,
                bill_key: Some(private_key_test()),
            },
            &keys,
            test_ts(),
        );
        assert!(sign_person_bill_block.is_ok());
        chain.try_add_block(sign_person_bill_block.unwrap());

        let sign_company_bill_block = IdentityBlock::create_block_for_sign_company_bill(
            chain.get_latest_block(),
            &IdentitySignCompanyBillBlockData {
                bill_id: bill_id_test(),
                block_id: BlockId::first(),
                block_hash: Sha256Hash::new("some hash"),
                company_id: node_id_test(),
                operation: BillOpCode::Issue,
            },
            &keys,
            test_ts(),
        );
        assert!(sign_company_bill_block.is_ok());
        chain.try_add_block(sign_company_bill_block.unwrap());

        let create_company_block = IdentityBlock::create_block_for_create_company(
            chain.get_latest_block(),
            &IdentityCreateCompanyBlockData {
                company_id: node_id_test(),
                company_key: private_key_test(),
                block_hash: Sha256Hash::new("some hash"),
            },
            &keys,
            test_ts(),
        );
        assert!(create_company_block.is_ok());
        chain.try_add_block(create_company_block.unwrap());

        let invite_signatory_block = IdentityBlock::create_block_for_invite_signatory(
            chain.get_latest_block(),
            &IdentityInviteSignatoryBlockData {
                company_id: node_id_test(),
                block_id: BlockId::next_from_previous_block_id(&BlockId::first()),
                block_hash: Sha256Hash::new("some hash"),
                signatory: node_id_test(),
            },
            &keys,
            test_ts(),
        );
        assert!(invite_signatory_block.is_ok());
        chain.try_add_block(invite_signatory_block.unwrap());

        let accept_signatory_invite_block = IdentityBlock::create_block_for_accept_signatory_invite(
            chain.get_latest_block(),
            &IdentityAcceptSignatoryInviteBlockData {
                company_id: node_id_test(),
                block_id: BlockId::next_from_previous_block_id(&BlockId::first()),
                block_hash: Sha256Hash::new("some hash"),
            },
            &keys,
            test_ts(),
        );
        assert!(accept_signatory_invite_block.is_ok());
        chain.try_add_block(accept_signatory_invite_block.unwrap());

        let reject_signatory_invite_block = IdentityBlock::create_block_for_reject_signatory_invite(
            chain.get_latest_block(),
            &IdentityRejectSignatoryInviteBlockData {
                company_id: node_id_test(),
                block_id: BlockId::next_from_previous_block_id(&BlockId::first()),
                block_hash: Sha256Hash::new("some hash"),
            },
            &keys,
            test_ts(),
        );
        assert!(reject_signatory_invite_block.is_ok());
        chain.try_add_block(reject_signatory_invite_block.unwrap());

        let remove_signatory_block = IdentityBlock::create_block_for_remove_signatory(
            chain.get_latest_block(),
            &IdentityRemoveSignatoryBlockData {
                company_id: node_id_test(),
                block_id: BlockId::next_from_previous_block_id(&BlockId::first()),
                block_hash: Sha256Hash::new("some hash"),
                signatory: node_id_test(),
            },
            &keys,
            test_ts(),
        );
        assert!(remove_signatory_block.is_ok());
        chain.try_add_block(remove_signatory_block.unwrap());

        let test_signed_identity = signed_identity_proof_test();
        let identity_proof_block = IdentityBlock::create_block_for_identity_proof(
            chain.get_latest_block(),
            &IdentityProofBlockData {
                proof: test_signed_identity.0,
                data: test_signed_identity.1,
            },
            &keys,
            test_ts(),
        );
        assert!(identity_proof_block.is_ok());
        chain.try_add_block(identity_proof_block.unwrap());

        assert_eq!(chain.blocks().len(), 10);
        assert!(chain.is_chain_valid());
        for block in chain.blocks() {
            assert!(block.validate_plaintext_hash(&keys.get_private_key()));
        }
    }
}
