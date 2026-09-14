use super::super::{Error, Result};
use super::BillOpCode;
use super::BillOpCode::{
    Accept, Endorse, Issue, Mint, OfferToSell, Recourse, RejectToAccept, RejectToBuy, RejectToPay,
    RejectToPayRecourse, RequestRecourse, RequestToAccept, RequestToPay, Sell,
};

use crate::protocol::Country;
use crate::protocol::Date;
use crate::protocol::Name;
use crate::protocol::SchnorrSignature;
use crate::protocol::Sha256Hash;
use crate::protocol::Sum;
use crate::protocol::Timestamp;
use crate::protocol::blockchain::Block;
use crate::protocol::blockchain::bill::participant::SignedBy;
use crate::protocol::blockchain::bill::{
    BillAction, BillHistoryBlock, BitcreditBill, RecourseReason,
};
use crate::protocol::constants::{
    ACCEPT_DEADLINE_SECONDS, PAYMENT_DEADLINE_SECONDS, RECOURSE_DEADLINE_SECONDS,
};
use crate::protocol::crypto::{self, BcrKeys};
use crate::protocol::{BlockId, Email};
use crate::protocol::{City, EmailIdentityProofData, SignedIdentityProof};

use crate::protocol::{BitcoinAddress, File, PostalAddress, ProtocolValidationError, Validate};
use bcr_common::core::{BillId, NodeId};
use bitcoin::base58;
use bitcoin::secp256k1::PublicKey;
use borsh::{from_slice, to_vec};
use borsh_derive::{BorshDeserialize, BorshSerialize};
use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BillBlock {
    pub bill_id: BillId,
    pub id: BlockId,
    pub plaintext_hash: Sha256Hash,
    pub hash: Sha256Hash,
    pub previous_hash: Sha256Hash,
    pub timestamp: Timestamp,
    pub data: Vec<u8>, // encrypted block data
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_pubkey",
        deserialize_with = "crate::protocol::serialization::deserialize_pubkey"
    )]
    pub public_key: PublicKey,
    pub signature: SchnorrSignature,
    pub op_code: BillOpCode,
}

#[derive(BorshSerialize)]
pub struct BillBlockDataToHash {
    pub bill_id: BillId,
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
    op_code: BillOpCode,
}

/// Data for reject to accept/pay/recourse
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRejectBlockData {
    pub rejecter: BillIdentParticipantBlockData, // reject to accept/pay/recourse has to be identified
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: PostalAddress,
    pub signer_identity_proof: BillSignerIdentityProofBlockdata,
}

/// Data for reject to buy
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRejectToBuyBlockData {
    pub rejecter: BillParticipantBlockData, // reject to buy can be done by anon
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>,
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillIssueBlockData {
    pub id: BillId,
    pub country_of_issuing: Country,
    pub city_of_issuing: City,
    pub drawee: BillIdentParticipantBlockData, // drawee always has to be identified
    pub drawer: BillIdentParticipantBlockData, // drawer always has to be identified
    pub payee: BillParticipantBlockData,       // payer can be anon
    pub sum: Sum,
    pub maturity_date: Date,
    pub issue_date: Date,
    pub country_of_payment: Country,
    pub city_of_payment: City,
    pub files: Vec<File>,
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: PostalAddress,
    pub signer_identity_proof: BillSignerIdentityProofBlockdata,
}

impl Validate for BillIssueBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.drawee.node_id == self.payee.node_id() {
            return Err(ProtocolValidationError::DraweeCantBePayee);
        }

        Ok(())
    }
}

impl BillIssueBlockData {
    pub fn from(
        value: BitcreditBill,
        signatory: Option<BillSignatoryBlockData>,
        timestamp: Timestamp,
        identity_proof: (SignedIdentityProof, EmailIdentityProofData),
    ) -> Self {
        let signing_address = value.drawer.postal_address.clone();
        Self {
            id: value.id,
            country_of_issuing: value.country_of_issuing,
            city_of_issuing: value.city_of_issuing,
            drawee: value.drawee.into(),
            drawer: value.drawer.into(),
            payee: value.payee.into(),
            sum: value.sum,
            maturity_date: value.maturity_date,
            issue_date: value.issue_date,
            country_of_payment: value.country_of_payment,
            city_of_payment: value.city_of_payment,
            files: value.files,
            signatory,
            signing_timestamp: timestamp,
            signing_address, // address of the issuer
            signer_identity_proof: identity_proof.into(),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillAcceptBlockData {
    pub accepter: BillIdentParticipantBlockData, // accepter is drawer and has to be identified
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: PostalAddress, // address of the accepter
    pub signer_identity_proof: BillSignerIdentityProofBlockdata,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRequestToPayBlockData {
    pub requester: BillParticipantBlockData, // requester is holder and can be anon
    pub payment_data: BillPaymentBlockData,
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the requester
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillRequestToPayBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        // The deadline has to be at or after the end of the day of signing time plus 48h
        // This means it's implicitly also at least 48h after end of day of maturity date
        // Since request to pay can't happen before start of day of maturity date
        let signing_ts_plus_minimum_deadline = self.signing_timestamp + PAYMENT_DEADLINE_SECONDS;
        if !signing_ts_plus_minimum_deadline
            .deadline_is_at_or_after_end_of_day_of(&self.payment_data.payment_deadline)
        {
            return Err(ProtocolValidationError::DeadlineBeforeMinimum);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRequestToAcceptBlockData {
    pub requester: BillParticipantBlockData, // requester is holder and can be anon
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the requester
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
    pub acceptance_deadline_timestamp: Timestamp,
}

impl Validate for BillRequestToAcceptBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        // The deadline has to be at or after the end of the day of signing time plus 48h
        let signing_ts_plus_minimum_deadline = self.signing_timestamp + ACCEPT_DEADLINE_SECONDS;
        if !signing_ts_plus_minimum_deadline
            .deadline_is_at_or_after_end_of_day_of(&self.acceptance_deadline_timestamp)
        {
            return Err(ProtocolValidationError::DeadlineBeforeMinimum);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillMintBlockData {
    pub endorser: BillParticipantBlockData, // bill can be minted by anon
    pub endorsee: BillParticipantBlockData, // mints can be anon
    pub sum: Sum,
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the endorser
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillMintBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.endorsee.node_id() == self.endorser.node_id() {
            return Err(ProtocolValidationError::EndorserCantBeEndorsee);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillOfferToSellBlockData {
    pub seller: BillParticipantBlockData, // seller is holder and can be anon
    pub buyer: BillParticipantBlockData,  // buyer can be anon
    pub payment_data: BillPaymentBlockData,
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the seller
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillOfferToSellBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.buyer.node_id() == self.seller.node_id() {
            return Err(ProtocolValidationError::BuyerCantBeSeller);
        }

        // The deadline has to be at or after the end of the day of signing time
        if !self
            .signing_timestamp
            .deadline_is_at_or_after_end_of_day_of(&self.payment_data.payment_deadline)
        {
            return Err(ProtocolValidationError::DeadlineBeforeMinimum);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillSellBlockData {
    pub seller: BillParticipantBlockData, // seller is holder and can be anon
    pub buyer: BillParticipantBlockData,  // buyer can be anon
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the seller
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillSellBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.buyer.node_id() == self.seller.node_id() {
            return Err(ProtocolValidationError::BuyerCantBeSeller);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillEndorseBlockData {
    pub endorser: BillParticipantBlockData, // endorser is holder and can be anon
    pub endorsee: BillParticipantBlockData, // endorsee can be anon
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the endorser
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillEndorseBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.endorsee.node_id() == self.endorser.node_id() {
            return Err(ProtocolValidationError::EndorserCantBeEndorsee);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRequestRecourseBlockData {
    pub recourser: BillParticipantBlockData, // anon can do recourse
    pub recoursee: BillIdentParticipantBlockData, // anon can't be recoursed against
    pub payment_data: BillPaymentBlockData,
    pub recourse_reason: BillRecourseReasonBlockData,
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the recourser
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum BillRecourseReasonBlockData {
    Accept,
    Pay,
}

impl Validate for BillRequestRecourseBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.recoursee.node_id == self.recourser.node_id() {
            return Err(ProtocolValidationError::RecourserCantBeRecoursee);
        }

        // The deadline has to be at or after the end of the day of signing time plus 48h
        let signing_ts_plus_minimum_deadline = self.signing_timestamp + RECOURSE_DEADLINE_SECONDS;
        if !signing_ts_plus_minimum_deadline
            .deadline_is_at_or_after_end_of_day_of(&self.payment_data.payment_deadline)
        {
            return Err(ProtocolValidationError::DeadlineBeforeMinimum);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillRecourseBlockData {
    pub recourser: BillParticipantBlockData, // anon can do recourse
    pub recoursee: BillIdentParticipantBlockData, // anon can't be recoursed against
    pub signatory: Option<BillSignatoryBlockData>,
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddress>, // address of the endorser
    pub signer_identity_proof: Option<BillSignerIdentityProofBlockdata>,
}

impl Validate for BillRecourseBlockData {
    fn validate(&self) -> std::result::Result<(), ProtocolValidationError> {
        if self.recoursee.node_id == self.recourser.node_id() {
            return Err(ProtocolValidationError::RecourserCantBeRecoursee);
        }

        Ok(())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillPaymentBlockData {
    pub sum: Sum,
    #[borsh(
        serialize_with = "crate::protocol::serialization::serialize_bitcoin_address",
        deserialize_with = "crate::protocol::serialization::deserialize_bitcoin_address"
    )]
    pub payment_address: BitcoinAddress,
    pub payment_deadline: Timestamp,
}

/// Participant in a bill transaction - either anonymous, or identified
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum BillParticipantBlockData {
    Anon(BillAnonParticipantBlockData),
    Ident(BillIdentParticipantBlockData),
}

impl BillParticipantBlockData {
    pub fn node_id(&self) -> NodeId {
        match self {
            BillParticipantBlockData::Anon(data) => data.node_id.clone(),
            BillParticipantBlockData::Ident(data) => data.node_id.clone(),
        }
    }

    pub fn is_anon(&self) -> bool {
        matches!(self, BillParticipantBlockData::Anon(_))
    }

    pub fn is_ident(&self) -> bool {
        matches!(self, BillParticipantBlockData::Ident(_))
    }
}

/// Anon bill participany data
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BillAnonParticipantBlockData {
    pub node_id: NodeId,
}

/// Legal data for parties of a bill within the liability chain
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct BillIdentParticipantBlockData {
    pub t: ContactType,
    pub node_id: NodeId,
    pub name: Name,
    pub postal_address: PostalAddress,
}

impl BillIdentParticipantBlockData {
    pub fn node_id(&self) -> NodeId {
        self.node_id.clone()
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
    Default,
)]
#[borsh(use_discriminant = true)]
pub enum ContactType {
    #[default]
    Person = 0,
    Company = 1,
    Anon = 2,
}

impl TryFrom<u64> for ContactType {
    type Error = ProtocolValidationError;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(ContactType::Person),
            1 => Ok(ContactType::Company),
            2 => Ok(ContactType::Anon),
            _ => Err(ProtocolValidationError::InvalidContactType),
        }
    }
}

/// The name and node_id of a company signatory
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillSignatoryBlockData {
    pub node_id: NodeId,
    pub name: Option<Name>, // only set if the signer the signatory is signing for is not an anon holder
}

/// The identity proof data, signature and witness for the signer of the block
#[derive(BorshSerialize, BorshDeserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BillSignerIdentityProofBlockdata {
    pub node_id: NodeId,
    pub company_node_id: Option<NodeId>,
    pub email: Email,
    pub created_at: Timestamp,
    pub signature: SchnorrSignature,
    pub witness: NodeId,
}

impl From<(SignedIdentityProof, EmailIdentityProofData)> for BillSignerIdentityProofBlockdata {
    fn from((proof, data): (SignedIdentityProof, EmailIdentityProofData)) -> Self {
        Self {
            node_id: data.node_id,
            company_node_id: data.company_node_id,
            email: data.email,
            created_at: data.created_at,
            signature: proof.signature,
            witness: proof.witness,
        }
    }
}

impl From<BillSignerIdentityProofBlockdata> for (SignedIdentityProof, EmailIdentityProofData) {
    fn from(
        value: BillSignerIdentityProofBlockdata,
    ) -> (SignedIdentityProof, EmailIdentityProofData) {
        (
            SignedIdentityProof {
                signature: value.signature,
                witness: value.witness,
            },
            EmailIdentityProofData {
                node_id: value.node_id,
                company_node_id: value.company_node_id,
                email: value.email,
                created_at: value.created_at,
            },
        )
    }
}

/// The data of the new holder in a holder-changing block, with the signatory data from the block
#[derive(Clone, Debug)]
pub struct HolderFromBlock {
    pub holder: BillParticipantBlockData, // holder can be anon
    pub signer: BillParticipantBlockData, // signer can be anon
    pub signatory: Option<BillSignatoryBlockData>,
}

impl Block for BillBlock {
    type OpCode = BillOpCode;
    type BlockDataToHash = BillBlockDataToHash;

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

    /// We validate the plaintext hash against the plaintext data from the BillBlockData wrapper
    fn validate_plaintext_hash(&self, private_key: &bitcoin::secp256k1::SecretKey) -> bool {
        match from_slice::<BillBlockData>(self.data()) {
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
        BillBlockDataToHash {
            bill_id: self.bill_id.clone(),
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

/// Structure for the block data of a bill block
///
/// - `data` contains the actual data of the block, encrypted using the bill's pub key
/// - `key` is optional and if set, contains the bill private key encrypted by an identity
///   pub key (e.g. for Issue the issuer's and Endorse the endorsee's)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct BillBlockData {
    data: Vec<u8>,
    // The encrypted, base58 encoded SecretKey
    key: Option<String>,
}

impl BillBlock {
    /// Create a new block and sign it with an aggregated key, combining the identity key of the
    /// signer, and the company key if it exists and the bill key
    pub fn new(
        bill_id: BillId,
        id: BlockId,
        previous_hash: Sha256Hash,
        data: Vec<u8>,
        op_code: BillOpCode,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
        plaintext_hash: Sha256Hash,
    ) -> Result<Self> {
        // The order here is important: identity -> company -> bill
        let mut keys: Vec<bitcoin::secp256k1::SecretKey> = vec![];
        keys.push(identity_keys.get_private_key());
        if let Some(company_key) = company_keys {
            keys.push(company_key.get_private_key());
        }
        keys.push(bill_keys.get_private_key());

        let aggregated_public_key = SchnorrSignature::get_aggregated_public_key(&keys)?;
        let hash = Self::calculate_hash(BillBlockDataToHash {
            bill_id: bill_id.clone(),
            id,
            plaintext_hash: plaintext_hash.clone(),
            previous_hash: previous_hash.clone(),
            data: data.clone(),
            timestamp,
            public_key: aggregated_public_key.to_owned(),
            op_code: op_code.clone(),
        })?;
        let signature = SchnorrSignature::sign_aggregated(&hash, &keys)?;

        Ok(Self {
            bill_id,
            id,
            plaintext_hash,
            hash,
            timestamp,
            previous_hash,
            signature,
            public_key: aggregated_public_key,
            data,
            op_code,
        })
    }

    pub fn create_block_for_issue(
        bill_id: BillId,
        genesis_hash: Sha256Hash,
        bill: &BillIssueBlockData,
        drawer_keys: &BcrKeys,
        drawer_company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let plaintext_hash = Self::calculate_plaintext_hash(bill)?;
        let key_bytes = to_vec(&bill_keys.get_private_key_string())?;
        // If drawer is a company, use drawer_company_keys for encryption
        let encrypted_key = match drawer_company_keys {
            None => base58::encode(&crypto::encrypt_ecies(&key_bytes, &drawer_keys.pub_key())?),
            Some(company_keys) => {
                base58::encode(&crypto::encrypt_ecies(&key_bytes, &company_keys.pub_key())?)
            }
        };

        let issue_data_bytes = to_vec(bill)?;

        let encrypted_data = crypto::encrypt_ecies(&issue_data_bytes, &bill_keys.pub_key())?;

        let data = BillBlockData {
            data: encrypted_data,
            key: Some(encrypted_key),
        };
        let serialized_data = to_vec(&data)?;

        Self::new(
            bill_id,
            BlockId::first(),
            genesis_hash,
            serialized_data,
            BillOpCode::Issue,
            drawer_keys,
            drawer_company_keys,
            bill_keys,
            timestamp,
            plaintext_hash,
        )
    }

    pub fn create_block_for_reject_to_accept(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRejectBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RejectToAccept,
        )?;
        Ok(block)
    }

    pub fn create_block_for_reject_to_pay(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRejectBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RejectToPay,
        )?;
        Ok(block)
    }

    pub fn create_block_for_reject_to_buy(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRejectToBuyBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RejectToBuy,
        )?;
        Ok(block)
    }

    pub fn create_block_for_reject_to_pay_recourse(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRejectBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RejectToPayRecourse,
        )?;
        Ok(block)
    }

    pub fn create_block_for_request_recourse(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRequestRecourseBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RequestRecourse,
        )?;
        Ok(block)
    }

    pub fn create_block_for_recourse(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRecourseBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::Recourse,
        )?;
        Ok(block)
    }

    pub fn create_block_for_accept(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillAcceptBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::Accept,
        )?;
        Ok(block)
    }

    pub fn create_block_for_request_to_pay(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRequestToPayBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RequestToPay,
        )?;
        Ok(block)
    }

    pub fn create_block_for_request_to_accept(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillRequestToAcceptBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::RequestToAccept,
        )?;
        Ok(block)
    }

    pub fn create_block_for_mint(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillMintBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            Some(data.endorsee.node_id().pub_key()),
            timestamp,
            BillOpCode::Mint,
        )?;
        Ok(block)
    }

    pub fn create_block_for_offer_to_sell(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillOfferToSellBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            None,
            timestamp,
            BillOpCode::OfferToSell,
        )?;
        Ok(block)
    }

    pub fn create_block_for_sell(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillSellBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            Some(data.buyer.node_id().pub_key()),
            timestamp,
            BillOpCode::Sell,
        )?;
        Ok(block)
    }

    pub fn create_block_for_endorse(
        bill_id: BillId,
        previous_block: &Self,
        data: &BillEndorseBlockData,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let block = Self::encrypt_data_create_block_and_validate(
            bill_id,
            previous_block,
            data,
            identity_keys,
            company_keys,
            bill_keys,
            Some(data.endorsee.node_id().pub_key()),
            timestamp,
            BillOpCode::Endorse,
        )?;
        Ok(block)
    }

    fn encrypt_data_create_block_and_validate<T: borsh::BorshSerialize>(
        bill_id: BillId,
        previous_block: &Self,
        data: &T,
        identity_keys: &BcrKeys,
        company_keys: Option<&BcrKeys>,
        bill_keys: &BcrKeys,
        public_key_for_keys: Option<PublicKey>, // when encrypting keys for a new holder
        timestamp: Timestamp,
        op_code: BillOpCode,
    ) -> Result<Self> {
        let bytes = to_vec(&data)?;
        let plaintext_hash = Self::calculate_plaintext_hash(data)?;
        // encrypt data using the bill pub key
        let encrypted_data = crypto::encrypt_ecies(&bytes, &bill_keys.pub_key())?;

        let mut key = None;

        // in case there are keys to encrypt, encrypt them using the receiver's identity pub key
        if (op_code == BillOpCode::Endorse
            || op_code == BillOpCode::Sell
            || op_code == BillOpCode::Mint)
            && let Some(new_holder_public_key) = public_key_for_keys
        {
            let key_bytes = to_vec(&bill_keys.get_private_key_string())?;
            let encrypted_key =
                base58::encode(&crypto::encrypt_ecies(&key_bytes, &new_holder_public_key)?);
            key = Some(encrypted_key);
        }

        let data = BillBlockData {
            data: encrypted_data,
            key,
        };
        let serialized_data = to_vec(&data)?;

        let new_block = Self::new(
            bill_id,
            BlockId::next_from_previous_block_id(&previous_block.id),
            previous_block.hash.clone(),
            serialized_data,
            op_code,
            identity_keys,
            company_keys,
            bill_keys,
            timestamp,
            plaintext_hash,
        )?;

        if !new_block.validate_with_previous(previous_block) {
            return Err(Error::BlockInvalid);
        }
        Ok(new_block)
    }

    /// Decrypts the block data using the bill's private key, returning the deserialized data
    pub fn get_decrypted_block<T: borsh::BorshDeserialize>(
        &self,
        bill_keys: &BcrKeys,
    ) -> Result<T> {
        let decrypted_bytes = self.get_decrypted_block_bytes(bill_keys)?;
        let deserialized = from_slice::<T>(&decrypted_bytes)?;
        Ok(deserialized)
    }

    /// Decrypts the block data using the bill's private key, returning the deserialized data
    pub(super) fn get_decrypted_block_bytes(&self, bill_keys: &BcrKeys) -> Result<Vec<u8>> {
        let block_data: BillBlockData = from_slice(&self.data)?;
        let decrypted_bytes =
            crypto::decrypt_ecies(&block_data.data, &bill_keys.get_private_key())?;
        Ok(decrypted_bytes)
    }

    /// Extracts a list of unique node IDs involved in a block operation.
    ///
    /// # Parameters
    /// - `bill_keys`: The bill's keys
    ///
    /// # Returns
    /// A `Vec<String>` containing the unique peer IDs involved in the block. Peer IDs are included
    /// only if they are non-empty.
    ///
    pub fn get_nodes_from_block(&self, bill_keys: &BcrKeys) -> Result<Vec<NodeId>> {
        let mut nodes = HashSet::new();
        match self.op_code {
            Issue => {
                let bill: BillIssueBlockData = self.get_decrypted_block(bill_keys)?;
                nodes.insert(bill.drawer.node_id);
                nodes.insert(bill.payee.node_id().to_owned());
                nodes.insert(bill.drawee.node_id);
            }
            Endorse => {
                let block_data_decrypted: BillEndorseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.endorsee.node_id());
                nodes.insert(block_data_decrypted.endorser.node_id());
            }
            Mint => {
                let block_data_decrypted: BillMintBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.endorsee.node_id());
                nodes.insert(block_data_decrypted.endorser.node_id());
            }
            RequestToAccept => {
                let block_data_decrypted: BillRequestToAcceptBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.requester.node_id());
            }
            Accept => {
                let block_data_decrypted: BillAcceptBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.accepter.node_id);
            }
            RequestToPay => {
                let block_data_decrypted: BillRequestToPayBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.requester.node_id());
            }
            OfferToSell => {
                let block_data_decrypted: BillOfferToSellBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.buyer.node_id());
                nodes.insert(block_data_decrypted.seller.node_id());
            }
            Sell => {
                let block_data_decrypted: BillSellBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.buyer.node_id());
                nodes.insert(block_data_decrypted.seller.node_id());
            }
            RejectToAccept | RejectToPay | RejectToPayRecourse => {
                let block_data_decrypted: BillRejectBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.rejecter.node_id);
            }
            RejectToBuy => {
                let block_data_decrypted: BillRejectToBuyBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.rejecter.node_id());
            }
            RequestRecourse => {
                let block_data_decrypted: BillRequestRecourseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.recourser.node_id());
                nodes.insert(block_data_decrypted.recoursee.node_id);
            }
            Recourse => {
                let block_data_decrypted: BillRecourseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                nodes.insert(block_data_decrypted.recourser.node_id());
                nodes.insert(block_data_decrypted.recoursee.node_id);
            }
        }
        Ok(nodes.into_iter().collect())
    }

    /// If the block is a holder-changing block with a financial beneficiary(sell, recourse),
    /// return the node_id of the beneficiary
    pub fn get_beneficiary_from_block(&self, bill_keys: &BcrKeys) -> Result<Option<NodeId>> {
        match self.op_code {
            Sell => {
                let block: BillSellBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(block.seller.node_id()))
            }
            Recourse => {
                let block: BillRecourseBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(block.recourser.node_id()))
            }
            _ => Ok(None),
        }
    }

    /// If the block is a request with a financial beneficiary(req to pay, offer to sell, req to recourse),
    /// return the node_id of the beneficiary
    pub fn get_beneficiary_from_request_funds_block(
        &self,
        bill_keys: &BcrKeys,
    ) -> Result<Option<NodeId>> {
        match self.op_code {
            OfferToSell => {
                let block: BillOfferToSellBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(block.seller.node_id()))
            }
            RequestRecourse => {
                let block: BillRequestRecourseBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(block.recourser.node_id()))
            }
            RequestToPay => {
                let block: BillRequestToPayBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(block.requester.node_id()))
            }
            _ => Ok(None),
        }
    }

    pub fn get_history_from_block(&self, bill_keys: &BcrKeys) -> Result<BillHistoryBlock> {
        Ok(match self.op_code {
            Issue => {
                let block_data_decrypted: BillIssueBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        BillParticipantBlockData::Ident(block_data_decrypted.drawer),
                        block_data_decrypted.signatory,
                    )),
                    Some(block_data_decrypted.signing_address),
                )
            }
            Endorse => {
                let block_data_decrypted: BillEndorseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(block_data_decrypted.endorsee.into()),
                    None,
                    None,
                    SignedBy::from((
                        block_data_decrypted.endorser,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            Mint => {
                let block_data_decrypted: BillMintBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(block_data_decrypted.endorsee.into()),
                    None,
                    None,
                    SignedBy::from((
                        block_data_decrypted.endorser,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            RequestToAccept => {
                let block_data_decrypted: BillRequestToAcceptBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    Some(block_data_decrypted.acceptance_deadline_timestamp),
                    SignedBy::from((
                        block_data_decrypted.requester,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            Accept => {
                let block_data_decrypted: BillAcceptBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        BillParticipantBlockData::Ident(block_data_decrypted.accepter),
                        block_data_decrypted.signatory,
                    )),
                    Some(block_data_decrypted.signing_address),
                )
            }
            RequestToPay => {
                let block_data_decrypted: BillRequestToPayBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    Some(block_data_decrypted.payment_data.clone().into()),
                    Some(block_data_decrypted.payment_data.payment_deadline),
                    SignedBy::from((
                        block_data_decrypted.requester,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            OfferToSell => {
                let block_data_decrypted: BillOfferToSellBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(block_data_decrypted.buyer.into()),
                    Some(block_data_decrypted.payment_data.clone().into()),
                    Some(block_data_decrypted.payment_data.payment_deadline),
                    SignedBy::from((block_data_decrypted.seller, block_data_decrypted.signatory)),
                    block_data_decrypted.signing_address,
                )
            }
            Sell => {
                let block_data_decrypted: BillSellBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(block_data_decrypted.buyer.into()),
                    None,
                    None,
                    SignedBy::from((block_data_decrypted.seller, block_data_decrypted.signatory)),
                    block_data_decrypted.signing_address,
                )
            }
            RejectToAccept => {
                let block_data_decrypted: BillRejectBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        BillParticipantBlockData::Ident(block_data_decrypted.rejecter),
                        block_data_decrypted.signatory,
                    )),
                    Some(block_data_decrypted.signing_address),
                )
            }
            RejectToPay => {
                let block_data_decrypted: BillRejectBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        BillParticipantBlockData::Ident(block_data_decrypted.rejecter),
                        block_data_decrypted.signatory,
                    )),
                    Some(block_data_decrypted.signing_address),
                )
            }
            RejectToPayRecourse => {
                let block_data_decrypted: BillRejectBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        BillParticipantBlockData::Ident(block_data_decrypted.rejecter),
                        block_data_decrypted.signatory,
                    )),
                    Some(block_data_decrypted.signing_address),
                )
            }
            RejectToBuy => {
                let block_data_decrypted: BillRejectToBuyBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    None,
                    None,
                    None,
                    SignedBy::from((
                        block_data_decrypted.rejecter,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            RequestRecourse => {
                let block_data_decrypted: BillRequestRecourseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(BillParticipantBlockData::Ident(block_data_decrypted.recoursee).into()),
                    Some(block_data_decrypted.payment_data.clone().into()),
                    Some(block_data_decrypted.payment_data.payment_deadline),
                    SignedBy::from((
                        block_data_decrypted.recourser,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
            Recourse => {
                let block_data_decrypted: BillRecourseBlockData =
                    self.get_decrypted_block(bill_keys)?;
                BillHistoryBlock::new(
                    self,
                    Some(BillParticipantBlockData::Ident(block_data_decrypted.recoursee).into()),
                    None,
                    None,
                    SignedBy::from((
                        block_data_decrypted.recourser,
                        block_data_decrypted.signatory,
                    )),
                    block_data_decrypted.signing_address,
                )
            }
        })
    }

    /// If the block is holder-changing block (issue, endorse, sell, mint, recourse), returns
    /// the new holder and signer data from the block
    pub fn get_holder_from_block(&self, bill_keys: &BcrKeys) -> Result<Option<HolderFromBlock>> {
        match self.op_code {
            Issue => {
                let bill: BillIssueBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(HolderFromBlock {
                    holder: bill.payee,
                    signer: BillParticipantBlockData::Ident(bill.drawer),
                    signatory: bill.signatory,
                }))
            }
            Endorse => {
                let block: BillEndorseBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(HolderFromBlock {
                    holder: block.endorsee,
                    signer: block.endorser,
                    signatory: block.signatory,
                }))
            }
            Mint => {
                let block: BillMintBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(HolderFromBlock {
                    holder: block.endorsee,
                    signer: block.endorser,
                    signatory: block.signatory,
                }))
            }
            Sell => {
                let block: BillSellBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(HolderFromBlock {
                    holder: block.buyer,
                    signer: block.seller,
                    signatory: block.signatory,
                }))
            }
            Recourse => {
                let block: BillRecourseBlockData = self.get_decrypted_block(bill_keys)?;
                Ok(Some(HolderFromBlock {
                    holder: BillParticipantBlockData::Ident(block.recoursee),
                    signer: block.recourser,
                    signatory: block.signatory,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Validates the block data and Verifies that the signer/signatory combo in the block is the one who signed the block and
    /// returns the signer_node_id and bill action for the block
    pub fn verify_and_get_signer(
        &self,
        bill_keys: &BcrKeys,
    ) -> Result<(NodeId, Option<BillAction>)> {
        let (signer, signatory, bill_action, identity_proof) = match self.op_code {
            Issue => {
                let data: BillIssueBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.drawer.node_id,
                    data.signatory.map(|s| s.node_id),
                    None,
                    Some(data.signer_identity_proof),
                )
            }
            Endorse => {
                let data: BillEndorseBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.endorser.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::Endorse(data.endorsee.into())),
                    data.signer_identity_proof,
                )
            }
            Mint => {
                let data: BillMintBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.endorser.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::Mint(data.endorsee.into(), data.sum)),
                    data.signer_identity_proof,
                )
            }
            RequestToAccept => {
                let data: BillRequestToAcceptBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.requester.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RequestAcceptance(
                        data.acceptance_deadline_timestamp,
                    )),
                    data.signer_identity_proof,
                )
            }
            Accept => {
                let data: BillAcceptBlockData = self.get_decrypted_block(bill_keys)?;
                // nothing to validate - all checked via type system
                (
                    data.accepter.node_id,
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::Accept),
                    Some(data.signer_identity_proof),
                )
            }
            RequestToPay => {
                let data: BillRequestToPayBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.requester.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RequestToPay(
                        data.payment_data.sum.currency().to_owned(),
                        data.payment_data.payment_deadline,
                        None,
                    )),
                    data.signer_identity_proof,
                )
            }
            OfferToSell => {
                let data: BillOfferToSellBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.seller.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::OfferToSell(
                        data.buyer.into(),
                        data.payment_data.sum,
                        data.payment_data.payment_deadline,
                    )),
                    data.signer_identity_proof,
                )
            }
            Sell => {
                let data: BillSellBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.seller.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::Sell(data.buyer.into())),
                    data.signer_identity_proof,
                )
            }
            RejectToAccept => {
                let data: BillRejectBlockData = self.get_decrypted_block(bill_keys)?;
                // nothing to validate - all checked via type system
                (
                    data.rejecter.node_id,
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RejectAcceptance),
                    Some(data.signer_identity_proof),
                )
            }
            RejectToBuy => {
                let data: BillRejectToBuyBlockData = self.get_decrypted_block(bill_keys)?;
                // nothing to validate - all checked via type system
                (
                    data.rejecter.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RejectBuying),
                    data.signer_identity_proof,
                )
            }
            RejectToPay => {
                let data: BillRejectBlockData = self.get_decrypted_block(bill_keys)?;
                // nothing to validate - all checked via type system
                (
                    data.rejecter.node_id,
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RejectPayment),
                    Some(data.signer_identity_proof),
                )
            }
            RejectToPayRecourse => {
                let data: BillRejectBlockData = self.get_decrypted_block(bill_keys)?;
                // nothing to validate - all checked via type system
                (
                    data.rejecter.node_id,
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RejectPaymentForRecourse),
                    Some(data.signer_identity_proof),
                )
            }
            RequestRecourse => {
                let data: BillRequestRecourseBlockData = self.get_decrypted_block(bill_keys)?;
                let reason = match data.recourse_reason {
                    BillRecourseReasonBlockData::Pay => {
                        RecourseReason::Pay(data.payment_data.sum.clone())
                    }
                    BillRecourseReasonBlockData::Accept => RecourseReason::Accept,
                };
                data.validate()?;
                (
                    data.recourser.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::RequestRecourse(
                        data.recoursee.into(),
                        reason,
                        data.payment_data.payment_deadline,
                    )),
                    data.signer_identity_proof,
                )
            }
            Recourse => {
                let data: BillRecourseBlockData = self.get_decrypted_block(bill_keys)?;
                data.validate()?;
                (
                    data.recourser.node_id(),
                    data.signatory.map(|s| s.node_id),
                    Some(BillAction::Recourse(data.recoursee.into())),
                    data.signer_identity_proof,
                )
            }
        };

        if !self.verify_signer(&signer, &signatory, bill_keys)? {
            return Err(Error::BlockSignatureDoesNotMatchSigner);
        }

        // If the identity proof doesn't match with the signer, or isn't valid, we show a warning
        if let Some(ip) = identity_proof {
            let (proof, data): (SignedIdentityProof, EmailIdentityProofData) = ip.into();
            if let Some(sig) = signatory {
                if data.node_id != sig || data.company_node_id != Some(signer.clone()) {
                    warn!(
                        "Identity Proof Verification failed for bill {}, block {} and signer {}, signatory {} - signatory and signer don't match with identity proof",
                        self.bill_id,
                        self.id(),
                        signer,
                        sig
                    );
                }
            } else if data.node_id != signer || data.company_node_id.is_some() {
                warn!(
                    "Identity Proof Verification failed for bill {}, block {} and signer {} -  signer doesn't match with identity proof",
                    self.bill_id,
                    self.id(),
                    signer,
                );
            }

            if !proof.verify(&data).unwrap_or(false) {
                warn!(
                    "Identity Proof Verification failed for bill {}, block {} and signer {}",
                    self.bill_id,
                    self.id(),
                    signer
                );
            }
        }

        Ok((signer, bill_action))
    }

    fn verify_signer(
        &self,
        signer: &NodeId,
        signatory: &Option<NodeId>,
        bill_keys: &BcrKeys,
    ) -> Result<bool> {
        let mut keys: Vec<PublicKey> = vec![];
        // if there is a company signatory, add that key first, since it's the identity key
        if let Some(signatory) = signatory {
            keys.push(signatory.pub_key());
        }
        // then, add the signer key
        keys.push(signer.pub_key());
        // finally, add the bill key
        keys.push(bill_keys.pub_key());
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::protocol::{
        Address, Country, Zip,
        blockchain::bill::{participant::BillParticipant, tests::get_baseline_identity},
        constants::DAY_IN_SECS,
        tests::tests::{
            bill_id_test, bill_identified_participant_only_node_id, bill_participant_only_node_id,
            empty_bill_identified_participant, empty_bitcredit_bill, get_bill_keys, node_id_test,
            node_id_test_other, private_key_test, signed_identity_proof_test, test_ts,
            valid_address, valid_payment_address_testnet,
        },
    };

    fn get_first_block() -> BillBlock {
        let mut bill = empty_bitcredit_bill();
        bill.id = bill_id_test();
        let mut drawer = empty_bill_identified_participant();
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let mut payer = empty_bill_identified_participant();
        let payer_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        payer.node_id = payer_node_id.clone();
        drawer.node_id = node_id.clone();

        bill.drawer = drawer.clone();
        bill.payee = BillParticipant::Ident(drawer.clone());
        bill.drawee = payer;

        BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("prevhash"),
            &BillIssueBlockData::from(bill, None, test_ts(), signed_identity_proof_test()),
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&get_bill_keys().get_private_key()),
            test_ts(),
        )
        .unwrap()
    }

    #[test]
    fn test_plaintext_hash() {
        let bill = empty_bitcredit_bill();
        let bill_keys = BcrKeys::new();
        let block = BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("genesis"),
            &BillIssueBlockData::from(bill, None, test_ts(), signed_identity_proof_test()),
            &BcrKeys::new(),
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        assert!(block.verify());
        assert!(block.validate_plaintext_hash(&bill_keys.get_private_key()));
    }

    #[test]
    fn signature_can_be_verified() {
        let block = BillBlock::new(
            bill_id_test(),
            BlockId::first(),
            Sha256Hash::new("prevhash"),
            Vec::new(),
            BillOpCode::Issue,
            &BcrKeys::new(),
            None,
            &BcrKeys::new(),
            test_ts(),
            Sha256Hash::new("some plaintext hash"),
        )
        .unwrap();
        assert!(block.verify());
    }

    #[test]
    fn get_nodes_from_block_issue() {
        let mut bill = empty_bitcredit_bill();
        let mut drawer = empty_bill_identified_participant();
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let mut payer = empty_bill_identified_participant();
        let payer_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        payer.node_id = payer_node_id.clone();
        drawer.node_id = node_id.clone();
        bill.drawer = drawer.clone();
        bill.payee = BillParticipant::Ident(drawer.clone());
        bill.drawee = payer;

        let block = BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("prevhash"),
            &BillIssueBlockData::from(bill, None, test_ts(), signed_identity_proof_test()),
            &BcrKeys::new(),
            None,
            &BcrKeys::from_private_key(&get_bill_keys().get_private_key()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&node_id));
        assert!(res.as_ref().unwrap().contains(&payer_node_id));
    }

    #[test]
    fn get_nodes_from_block_endorse() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let endorsee = bill_participant_only_node_id(node_id.clone());
        let endorser = bill_participant_only_node_id(get_baseline_identity().0);
        let block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            &get_first_block(),
            &BillEndorseBlockData {
                endorser: endorser.clone().into(),
                endorsee: endorsee.into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&node_id));
        assert!(res.as_ref().unwrap().contains(&endorser.node_id()));
    }

    #[test]
    fn get_nodes_from_block_mint() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let mint = bill_participant_only_node_id(node_id.clone());
        let minter_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let minter = bill_participant_only_node_id(minter_node_id.clone());
        let block = BillBlock::create_block_for_mint(
            bill_id_test(),
            &get_first_block(),
            &BillMintBlockData {
                endorser: minter.clone().into(),
                endorsee: mint.into(),
                sum: Sum::new_sat(5000).expect("sat works"),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&node_id));
        assert!(res.as_ref().unwrap().contains(&minter_node_id));
    }

    #[test]
    fn get_nodes_from_block_req_to_accept() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let requester = bill_participant_only_node_id(node_id.clone());

        let block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            &get_first_block(),
            &BillRequestToAcceptBlockData {
                requester: requester.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: test_ts() + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&node_id));
    }

    #[test]
    fn get_nodes_from_block_accept() {
        let mut accepter = empty_bill_identified_participant();
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        accepter.node_id = node_id.clone();
        accepter.postal_address = PostalAddress {
            country: Country::AT,
            city: City::new("Vienna").unwrap(),
            zip: Some(Zip::new("1020").unwrap()),
            address: Address::new("Hayekweg 12").unwrap(),
        };

        let block = BillBlock::create_block_for_accept(
            bill_id_test(),
            &get_first_block(),
            &BillAcceptBlockData {
                accepter: accepter.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: accepter.postal_address,
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&node_id));
    }

    #[test]
    fn get_nodes_from_block_req_to_pay() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let requester = bill_participant_only_node_id(node_id.clone());

        let block = BillBlock::create_block_for_request_to_pay(
            bill_id_test(),
            &get_first_block(),
            &BillRequestToPayBlockData {
                requester: requester.clone().into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(500).unwrap(),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * PAYMENT_DEADLINE_SECONDS,
                },
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&node_id));
    }

    #[test]
    fn get_nodes_from_block_offer_to_sell() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let buyer = bill_participant_only_node_id(node_id.clone());
        let seller_node_id = get_baseline_identity().0;
        let seller = bill_participant_only_node_id(seller_node_id.clone());
        let block = BillBlock::create_block_for_offer_to_sell(
            bill_id_test(),
            &get_first_block(),
            &BillOfferToSellBlockData {
                buyer: buyer.clone().into(),
                seller: seller.clone().into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(5000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&node_id));
        assert!(res.as_ref().unwrap().contains(&seller_node_id));
    }

    #[test]
    fn get_nodes_from_block_sell() {
        let node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let buyer = bill_participant_only_node_id(node_id.clone());
        let seller_node_id = get_baseline_identity().0;
        let seller = bill_participant_only_node_id(seller_node_id.clone());
        let block = BillBlock::create_block_for_sell(
            bill_id_test(),
            &get_first_block(),
            &BillSellBlockData {
                buyer: buyer.clone().into(),
                seller: seller.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: buyer.node_id().clone(),
                    name: Some(Name::new("some name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(valid_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&node_id));
        assert!(res.as_ref().unwrap().contains(&seller_node_id));
    }

    #[test]
    fn get_nodes_from_block_reject_to_accept() {
        let rejecter = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_reject_to_accept(
            bill_id_test(),
            &get_first_block(),
            &BillRejectBlockData {
                rejecter: rejecter.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: rejecter.postal_address,
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&rejecter.node_id));
    }

    #[test]
    fn get_nodes_from_block_reject_to_pay() {
        let rejecter = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_reject_to_pay(
            bill_id_test(),
            &get_first_block(),
            &BillRejectBlockData {
                rejecter: rejecter.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: rejecter.postal_address,
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&rejecter.node_id));
    }

    #[test]
    fn get_nodes_from_block_reject_to_buy() {
        let rejecter = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_reject_to_buy(
            bill_id_test(),
            &get_first_block(),
            &BillRejectToBuyBlockData {
                rejecter: BillParticipant::Ident(rejecter.clone()).into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(rejecter.postal_address),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&rejecter.node_id));
    }

    #[test]
    fn get_nodes_from_block_reject_to_pay_recourse() {
        let rejecter = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_reject_to_pay_recourse(
            bill_id_test(),
            &get_first_block(),
            &BillRejectBlockData {
                rejecter: rejecter.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: rejecter.postal_address,
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert!(res.as_ref().unwrap().contains(&rejecter.node_id));
    }

    #[test]
    fn get_nodes_from_block_request_recourse() {
        let recoursee = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let recourser = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_request_recourse(
            bill_id_test(),
            &get_first_block(),
            &BillRequestRecourseBlockData {
                recourser: BillParticipant::Ident(recourser.clone()).into(),
                recoursee: recoursee.clone().into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(15000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * RECOURSE_DEADLINE_SECONDS,
                },
                recourse_reason: BillRecourseReasonBlockData::Pay,
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(recourser.postal_address),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&recourser.node_id));
        assert!(res.as_ref().unwrap().contains(&recoursee.node_id));
    }

    #[test]
    fn get_nodes_from_block_recourse() {
        let recoursee = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let recourser = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        let block = BillBlock::create_block_for_recourse(
            bill_id_test(),
            &get_first_block(),
            &BillRecourseBlockData {
                recourser: BillParticipant::Ident(recourser.clone()).into(),
                recoursee: recoursee.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(recourser.postal_address),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &get_baseline_identity().1,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
        )
        .unwrap();
        let res = block.get_nodes_from_block(&get_bill_keys());
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
        assert!(res.as_ref().unwrap().contains(&recourser.node_id));
        assert!(res.as_ref().unwrap().contains(&recoursee.node_id));
    }

    #[test]
    fn verify_and_get_signer_baseline() {
        let bill_keys = BcrKeys::new();
        let identity_keys = BcrKeys::new();
        let bill_keys_obj = BcrKeys::from_private_key(&bill_keys.get_private_key());

        let mut bill = empty_bitcredit_bill();
        let signer = bill_identified_participant_only_node_id(NodeId::new(
            identity_keys.pub_key(),
            bitcoin::Network::Testnet,
        ));
        let other_party = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        bill.drawer = signer.clone();
        bill.drawee = signer.clone();
        bill.payee = BillParticipant::Ident(other_party.clone());

        let issue_block = BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("genesis"),
            &BillIssueBlockData::from(bill, None, test_ts(), signed_identity_proof_test()),
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let issue_result = issue_block.verify_and_get_signer(&bill_keys_obj);
        assert!(issue_result.is_ok());
        assert_eq!(
            issue_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(issue_result.as_ref().unwrap().1.is_none());
        assert!(issue_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let endorse_block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            &issue_block,
            &BillEndorseBlockData {
                endorser: BillParticipant::Ident(signer.clone()).into(),
                endorsee: BillParticipant::Ident(other_party.clone()).into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let endorse_result = endorse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(endorse_result.is_ok());
        assert_eq!(
            endorse_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            endorse_result.as_ref().unwrap().1,
            Some(BillAction::Endorse(_))
        ));
        assert!(endorse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let mint_block = BillBlock::create_block_for_mint(
            bill_id_test(),
            &issue_block,
            &BillMintBlockData {
                endorser: BillParticipant::Ident(signer.clone()).into(),
                endorsee: BillParticipant::Ident(other_party.clone()).into(),
                sum: Sum::new_sat(5000).expect("sat works"),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let mint_result = mint_block.verify_and_get_signer(&bill_keys_obj);
        assert!(mint_result.is_ok());
        assert_eq!(
            mint_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            mint_result.as_ref().unwrap().1,
            Some(BillAction::Mint(_, _))
        ));
        assert!(mint_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let req_to_accept_block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            &issue_block,
            &BillRequestToAcceptBlockData {
                requester: BillParticipant::Ident(signer.clone()).into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: test_ts() + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let req_to_accept_result = req_to_accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(req_to_accept_result.is_ok());
        assert_eq!(
            req_to_accept_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            req_to_accept_result.as_ref().unwrap().1,
            Some(BillAction::RequestAcceptance(_))
        ));
        assert!(req_to_accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let req_to_pay_block = BillBlock::create_block_for_request_to_pay(
            bill_id_test(),
            &issue_block,
            &BillRequestToPayBlockData {
                requester: BillParticipant::Ident(signer.clone()).into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(500).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * PAYMENT_DEADLINE_SECONDS,
                },
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let req_to_pay_result = req_to_pay_block.verify_and_get_signer(&bill_keys_obj);
        assert!(req_to_pay_result.is_ok());
        assert_eq!(
            req_to_pay_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            req_to_pay_result.as_ref().unwrap().1,
            Some(BillAction::RequestToPay(_, _, _))
        ));
        assert!(req_to_pay_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let accept_block = BillBlock::create_block_for_accept(
            bill_id_test(),
            &issue_block,
            &BillAcceptBlockData {
                accepter: signer.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let accept_result = accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(accept_result.is_ok());
        assert_eq!(
            accept_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            accept_result.as_ref().unwrap().1,
            Some(BillAction::Accept)
        ));
        assert!(accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let offer_to_sell_block = BillBlock::create_block_for_offer_to_sell(
            bill_id_test(),
            &issue_block,
            &BillOfferToSellBlockData {
                seller: BillParticipant::Ident(signer.clone()).into(),
                buyer: BillParticipant::Ident(other_party.clone()).into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(5000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * DAY_IN_SECS,
                },
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let offer_to_sell_result = offer_to_sell_block.verify_and_get_signer(&bill_keys_obj);
        assert!(offer_to_sell_result.is_ok());
        assert_eq!(
            offer_to_sell_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            offer_to_sell_result.as_ref().unwrap().1,
            Some(BillAction::OfferToSell(_, _, _))
        ));
        assert!(offer_to_sell_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let sell_block = BillBlock::create_block_for_sell(
            bill_id_test(),
            &issue_block,
            &BillSellBlockData {
                seller: BillParticipant::Ident(signer.clone()).into(),
                buyer: BillParticipant::Ident(other_party.clone()).into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let sell_result = sell_block.verify_and_get_signer(&bill_keys_obj);
        assert!(sell_result.is_ok());
        assert_eq!(
            sell_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            sell_result.as_ref().unwrap().1,
            Some(BillAction::Sell(_))
        ));
        assert!(sell_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_accept_block = BillBlock::create_block_for_reject_to_accept(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_accept_result = reject_to_accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_accept_result.is_ok());
        assert_eq!(
            reject_to_accept_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_accept_result.as_ref().unwrap().1,
            Some(BillAction::RejectAcceptance)
        ));
        assert!(reject_to_accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_buy_block = BillBlock::create_block_for_reject_to_buy(
            bill_id_test(),
            &issue_block,
            &BillRejectToBuyBlockData {
                rejecter: BillParticipant::Ident(signer.clone()).into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_buy_result = reject_to_buy_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_buy_result.is_ok());
        assert_eq!(
            reject_to_buy_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_buy_result.as_ref().unwrap().1,
            Some(BillAction::RejectBuying)
        ));
        assert!(reject_to_buy_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_pay_block = BillBlock::create_block_for_reject_to_pay(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_pay_result = reject_to_pay_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_pay_result.is_ok());
        assert_eq!(
            reject_to_pay_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_pay_result.as_ref().unwrap().1,
            Some(BillAction::RejectPayment)
        ));
        assert!(reject_to_pay_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_pay_recourse_block = BillBlock::create_block_for_reject_to_pay_recourse(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_pay_recourse_result =
            reject_to_pay_recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_pay_recourse_result.is_ok());
        assert_eq!(
            reject_to_pay_recourse_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_pay_recourse_result.as_ref().unwrap().1,
            Some(BillAction::RejectPaymentForRecourse)
        ));
        assert!(reject_to_pay_recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let request_recourse_block = BillBlock::create_block_for_request_recourse(
            bill_id_test(),
            &issue_block,
            &BillRequestRecourseBlockData {
                recourser: BillParticipant::Ident(signer.clone()).into(),
                recoursee: other_party.clone().into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(15000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * RECOURSE_DEADLINE_SECONDS,
                },
                recourse_reason: BillRecourseReasonBlockData::Accept,
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let request_recourse_result = request_recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(request_recourse_result.is_ok());
        assert_eq!(
            request_recourse_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            request_recourse_result.as_ref().unwrap().1,
            Some(BillAction::RequestRecourse(_, _, _))
        ));
        assert!(request_recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let recourse_block = BillBlock::create_block_for_recourse(
            bill_id_test(),
            &issue_block,
            &BillRecourseBlockData {
                recourser: BillParticipant::Ident(signer.clone()).into(),
                recoursee: other_party.clone().into(),
                signatory: None,
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            None,
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let recourse_result = recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(recourse_result.is_ok());
        assert_eq!(
            recourse_result.as_ref().unwrap().0,
            NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            recourse_result.as_ref().unwrap().1,
            Some(BillAction::Recourse(_))
        ));
        assert!(recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));
    }

    #[test]
    fn verify_and_get_signer_baseline_company() {
        let bill_keys = BcrKeys::new();
        let company_keys = BcrKeys::new();
        let identity_keys = BcrKeys::new();
        let bill_keys_obj = BcrKeys::from_private_key(&bill_keys.get_private_key());

        let mut bill = empty_bitcredit_bill();
        let signer = bill_identified_participant_only_node_id(NodeId::new(
            company_keys.pub_key(),
            bitcoin::Network::Testnet,
        ));
        let other_party = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        ));
        bill.drawer = signer.clone();
        bill.drawee = signer.clone();
        bill.payee = BillParticipant::Ident(other_party.clone());

        let issue_block = BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("genesis"),
            &BillIssueBlockData::from(
                bill,
                Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                test_ts(),
                signed_identity_proof_test(),
            ),
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();

        let issue_result = issue_block.verify_and_get_signer(&bill_keys_obj);
        assert!(issue_result.is_ok());
        assert_eq!(
            issue_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(issue_result.as_ref().unwrap().1.is_none());
        assert!(issue_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let endorse_block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            &issue_block,
            &BillEndorseBlockData {
                endorser: BillParticipant::Ident(signer.clone()).into(),
                endorsee: BillParticipant::Ident(other_party.clone()).into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let endorse_result = endorse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(endorse_result.is_ok());
        assert_eq!(
            endorse_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            endorse_result.as_ref().unwrap().1,
            Some(BillAction::Endorse(_))
        ));
        assert!(endorse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let mint_block = BillBlock::create_block_for_mint(
            bill_id_test(),
            &issue_block,
            &BillMintBlockData {
                endorser: BillParticipant::Ident(signer.clone()).into(),
                endorsee: BillParticipant::Ident(other_party.clone()).into(),
                sum: Sum::new_sat(5000).expect("sat works"),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let mint_result = mint_block.verify_and_get_signer(&bill_keys_obj);
        assert!(mint_result.is_ok());
        assert_eq!(
            mint_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            mint_result.as_ref().unwrap().1,
            Some(BillAction::Mint(_, _))
        ));
        assert!(mint_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let req_to_accept_block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            &issue_block,
            &BillRequestToAcceptBlockData {
                requester: BillParticipant::Ident(signer.clone()).into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: test_ts() + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let req_to_accept_result = req_to_accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(req_to_accept_result.is_ok());
        assert_eq!(
            req_to_accept_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            req_to_accept_result.as_ref().unwrap().1,
            Some(BillAction::RequestAcceptance(_))
        ));
        assert!(req_to_accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let req_to_pay_block = BillBlock::create_block_for_request_to_pay(
            bill_id_test(),
            &issue_block,
            &BillRequestToPayBlockData {
                requester: BillParticipant::Ident(signer.clone()).into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(500).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * PAYMENT_DEADLINE_SECONDS,
                },
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let req_to_pay_result = req_to_pay_block.verify_and_get_signer(&bill_keys_obj);
        assert!(req_to_pay_result.is_ok());
        assert_eq!(
            req_to_pay_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            req_to_pay_result.as_ref().unwrap().1,
            Some(BillAction::RequestToPay(_, _, _))
        ));
        assert!(req_to_pay_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let accept_block = BillBlock::create_block_for_accept(
            bill_id_test(),
            &issue_block,
            &BillAcceptBlockData {
                accepter: signer.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let accept_result = accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(accept_result.is_ok());
        assert_eq!(
            accept_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            accept_result.as_ref().unwrap().1,
            Some(BillAction::Accept)
        ));
        assert!(accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let offer_to_sell_block = BillBlock::create_block_for_offer_to_sell(
            bill_id_test(),
            &issue_block,
            &BillOfferToSellBlockData {
                seller: BillParticipant::Ident(signer.clone()).into(),
                buyer: BillParticipant::Ident(other_party.clone()).into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(5000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * DAY_IN_SECS,
                },
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let offer_to_sell_result = offer_to_sell_block.verify_and_get_signer(&bill_keys_obj);
        assert!(offer_to_sell_result.is_ok());
        assert_eq!(
            offer_to_sell_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            offer_to_sell_result.as_ref().unwrap().1,
            Some(BillAction::OfferToSell(_, _, _))
        ));
        assert!(offer_to_sell_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let sell_block = BillBlock::create_block_for_sell(
            bill_id_test(),
            &issue_block,
            &BillSellBlockData {
                seller: BillParticipant::Ident(signer.clone()).into(),
                buyer: BillParticipant::Ident(other_party.clone()).into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let sell_result = sell_block.verify_and_get_signer(&bill_keys_obj);
        assert!(sell_result.is_ok());
        assert_eq!(
            sell_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            sell_result.as_ref().unwrap().1,
            Some(BillAction::Sell(_))
        ));
        assert!(sell_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_accept_block = BillBlock::create_block_for_reject_to_accept(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_accept_result = reject_to_accept_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_accept_result.is_ok());
        assert_eq!(
            reject_to_accept_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_accept_result.as_ref().unwrap().1,
            Some(BillAction::RejectAcceptance)
        ));
        assert!(reject_to_accept_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_buy_block = BillBlock::create_block_for_reject_to_buy(
            bill_id_test(),
            &issue_block,
            &BillRejectToBuyBlockData {
                rejecter: BillParticipant::Ident(signer.clone()).into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_buy_result = reject_to_buy_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_buy_result.is_ok());
        assert_eq!(
            reject_to_buy_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_buy_result.as_ref().unwrap().1,
            Some(BillAction::RejectBuying)
        ));
        assert!(reject_to_buy_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_pay_block = BillBlock::create_block_for_reject_to_pay(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_pay_result = reject_to_pay_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_pay_result.is_ok());
        assert_eq!(
            reject_to_pay_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_pay_result.as_ref().unwrap().1,
            Some(BillAction::RejectPayment)
        ));
        assert!(reject_to_pay_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let reject_to_pay_recourse_block = BillBlock::create_block_for_reject_to_pay_recourse(
            bill_id_test(),
            &issue_block,
            &BillRejectBlockData {
                rejecter: signer.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: signer.postal_address.clone(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let reject_to_pay_recourse_result =
            reject_to_pay_recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(reject_to_pay_recourse_result.is_ok());
        assert_eq!(
            reject_to_pay_recourse_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            reject_to_pay_recourse_result.as_ref().unwrap().1,
            Some(BillAction::RejectPaymentForRecourse)
        ));
        assert!(reject_to_pay_recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let request_recourse_block = BillBlock::create_block_for_request_recourse(
            bill_id_test(),
            &issue_block,
            &BillRequestRecourseBlockData {
                recourser: BillParticipant::Ident(signer.clone()).into(),
                recoursee: other_party.clone().into(),
                payment_data: BillPaymentBlockData {
                    sum: Sum::new_sat(15000).expect("sat works"),
                    payment_address: valid_payment_address_testnet(),
                    payment_deadline: test_ts() + 2 * RECOURSE_DEADLINE_SECONDS,
                },
                recourse_reason: BillRecourseReasonBlockData::Accept,
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let request_recourse_result = request_recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(request_recourse_result.is_ok());
        assert_eq!(
            request_recourse_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            request_recourse_result.as_ref().unwrap().1,
            Some(BillAction::RequestRecourse(_, _, _))
        ));
        assert!(request_recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));

        let recourse_block = BillBlock::create_block_for_recourse(
            bill_id_test(),
            &issue_block,
            &BillRecourseBlockData {
                recourser: BillParticipant::Ident(signer.clone()).into(),
                recoursee: other_party.clone().into(),
                signatory: Some(BillSignatoryBlockData {
                    node_id: NodeId::new(identity_keys.pub_key(), bitcoin::Network::Testnet),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                signing_timestamp: test_ts(),
                signing_address: Some(signer.postal_address.clone()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();
        let recourse_result = recourse_block.verify_and_get_signer(&bill_keys_obj);
        assert!(recourse_result.is_ok());
        assert_eq!(
            recourse_result.as_ref().unwrap().0,
            NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet)
        );
        assert!(matches!(
            recourse_result.as_ref().unwrap().1,
            Some(BillAction::Recourse(_))
        ));
        assert!(recourse_block.validate_plaintext_hash(&bill_keys.get_private_key()));
    }

    #[test]
    fn verify_and_get_signer_baseline_wrong_key() {
        let bill_keys = BcrKeys::new();
        let company_keys = BcrKeys::new();
        let identity_keys = BcrKeys::new();
        let bill_keys_obj = BcrKeys::from_private_key(&bill_keys.get_private_key());

        let mut bill = empty_bitcredit_bill();
        bill.drawer = bill_identified_participant_only_node_id(NodeId::new(
            BcrKeys::new().pub_key(),
            bitcoin::Network::Testnet,
        )); //company is drawer

        let block = BillBlock::create_block_for_issue(
            bill_id_test(),
            Sha256Hash::new("genesis"),
            &BillIssueBlockData::from(
                bill,
                Some(BillSignatoryBlockData {
                    node_id: node_id_test(),
                    name: Some(Name::new("signatory name").unwrap()),
                }),
                test_ts(),
                signed_identity_proof_test(),
            ),
            &identity_keys,
            Some(&company_keys),
            &bill_keys,
            test_ts(),
        )
        .unwrap();

        let result = block.verify_and_get_signer(&bill_keys_obj);
        assert!(result.is_err());
    }

    // Validation
    fn valid_bill_participant_block_data() -> BillParticipantBlockData {
        BillParticipantBlockData::Ident(BillIdentParticipantBlockData {
            t: ContactType::Person,
            node_id: node_id_test(),
            name: Name::new("Johanna Smith").unwrap(),
            postal_address: valid_address(),
        })
    }

    fn other_valid_bill_participant_block_data() -> BillParticipantBlockData {
        BillParticipantBlockData::Ident(BillIdentParticipantBlockData {
            t: ContactType::Person,
            node_id: node_id_test_other(),
            name: Name::new("John Smith").unwrap(),
            postal_address: valid_address(),
        })
    }

    fn valid_bill_identity_block_data() -> BillIdentParticipantBlockData {
        BillIdentParticipantBlockData {
            t: ContactType::Person,
            node_id: node_id_test(),
            name: Name::new("Johanna Smith").unwrap(),
            postal_address: valid_address(),
        }
    }

    fn other_valid_bill_identity_block_data() -> BillIdentParticipantBlockData {
        BillIdentParticipantBlockData {
            t: ContactType::Person,
            node_id: node_id_test_other(),
            name: Name::new("John Smith").unwrap(),
            postal_address: valid_address(),
        }
    }

    fn valid_bill_signatory_block_data() -> BillSignatoryBlockData {
        BillSignatoryBlockData {
            node_id: node_id_test(),
            name: Some(Name::new("Johanna Smith").unwrap()),
        }
    }

    pub fn valid_bill_issue_block_data() -> BillIssueBlockData {
        BillIssueBlockData {
            id: bill_id_test(),
            country_of_issuing: Country::AT,
            city_of_issuing: City::new("Vienna").unwrap(),
            drawee: other_valid_bill_identity_block_data(),
            drawer: valid_bill_identity_block_data(),
            payee: valid_bill_participant_block_data(),
            sum: Sum::new_sat(500).expect("sat works"),
            maturity_date: Date::new("2025-11-12").unwrap(),
            issue_date: Date::new("2025-08-12").unwrap(),
            country_of_payment: Country::FR,
            city_of_payment: City::new("Paris").unwrap(),
            files: vec![],
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: valid_address(),
            signer_identity_proof: signed_identity_proof_test().into(),
        }
    }

    #[test]
    fn test_valid_bill_issue_block_data() {
        let bill = valid_bill_issue_block_data();
        assert_eq!(bill.validate(), Ok(()));
    }

    fn valid_req_to_accept_block_data() -> BillRequestToAcceptBlockData {
        BillRequestToAcceptBlockData {
            requester: valid_bill_participant_block_data(),
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
            acceptance_deadline_timestamp: test_ts() + 2 * ACCEPT_DEADLINE_SECONDS,
        }
    }

    #[test]
    fn test_valid_req_to_accept_block_data() {
        let accept = valid_req_to_accept_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_req_to_pay_block_data() -> BillRequestToPayBlockData {
        BillRequestToPayBlockData {
            requester: valid_bill_participant_block_data(),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(500).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: test_ts() + 2 * PAYMENT_DEADLINE_SECONDS,
            },
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_req_to_pay_block_data() {
        let accept = valid_req_to_pay_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_mint_block_data() -> BillMintBlockData {
        BillMintBlockData {
            endorser: valid_bill_participant_block_data(),
            endorsee: other_valid_bill_participant_block_data(),
            sum: Sum::new_sat(500).expect("sat works"),
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_mint_block_data() {
        let accept = valid_mint_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_offer_to_sell_block_data() -> BillOfferToSellBlockData {
        BillOfferToSellBlockData {
            seller: valid_bill_participant_block_data(),
            buyer: other_valid_bill_participant_block_data(),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(500).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: test_ts() + 2 * DAY_IN_SECS,
            },
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_offer_to_sell_block_data() {
        let accept = valid_offer_to_sell_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_sell_block_data() -> BillSellBlockData {
        BillSellBlockData {
            seller: valid_bill_participant_block_data(),
            buyer: other_valid_bill_participant_block_data(),
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_sell_block_data() {
        let accept = valid_sell_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_endorse_block_data() -> BillEndorseBlockData {
        BillEndorseBlockData {
            endorser: valid_bill_participant_block_data(),
            endorsee: other_valid_bill_participant_block_data(),
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_endorse_block_data() {
        let accept = valid_endorse_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_req_to_recourse_block_data() -> BillRequestRecourseBlockData {
        BillRequestRecourseBlockData {
            recourser: valid_bill_participant_block_data(),
            recoursee: other_valid_bill_identity_block_data(),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(500).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: test_ts() + 2 * RECOURSE_DEADLINE_SECONDS,
            },
            recourse_reason: BillRecourseReasonBlockData::Pay,
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_req_to_recourse_block_data() {
        let accept = valid_req_to_recourse_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    fn valid_recourse_block_data() -> BillRecourseBlockData {
        BillRecourseBlockData {
            recourser: BillParticipantBlockData::Ident(valid_bill_identity_block_data()),
            recoursee: other_valid_bill_identity_block_data(),
            signatory: Some(valid_bill_signatory_block_data()),
            signing_timestamp: test_ts(),
            signing_address: Some(valid_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        }
    }

    #[test]
    fn test_valid_recourse_block_data() {
        let accept = valid_recourse_block_data();
        assert_eq!(accept.validate(), Ok(()));
    }

    #[test]
    fn test_truncate_from_mid_chain() {
        use crate::protocol::blockchain::Blockchain;
        use crate::protocol::blockchain::bill::chain::BillBlockchain;

        let keys = BcrKeys::new();
        let bill_keys = BcrKeys::new();
        let ts = test_ts();

        let block1 = BillBlock::new(
            bill_id_test(),
            BlockId::first(),
            Sha256Hash::new("genesis"),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts,
            Sha256Hash::new("ph1"),
        )
        .unwrap();

        let block2 = BillBlock::new(
            bill_id_test(),
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts + 1,
            Sha256Hash::new("ph2"),
        )
        .unwrap();

        let block3 = BillBlock::new(
            bill_id_test(),
            BlockId::next_from_previous_block_id(&block2.id),
            block2.hash.clone(),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts + 2,
            Sha256Hash::new("ph3"),
        )
        .unwrap();

        let mut chain =
            BillBlockchain::new_from_blocks(vec![block1.clone(), block2, block3]).unwrap();
        assert_eq!(chain.blocks().len(), 3);

        chain.truncate_from(BlockId::next_from_previous_block_id(&BlockId::first()));
        assert_eq!(chain.blocks().len(), 1);
        assert_eq!(chain.blocks()[0].id, block1.id);
    }

    #[test]
    fn test_truncate_from_genesis() {
        use crate::protocol::blockchain::Blockchain;
        use crate::protocol::blockchain::bill::chain::BillBlockchain;

        let keys = BcrKeys::new();
        let bill_keys = BcrKeys::new();
        let ts = test_ts();

        let block1 = BillBlock::new(
            bill_id_test(),
            BlockId::first(),
            Sha256Hash::new("genesis"),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts,
            Sha256Hash::new("ph1"),
        )
        .unwrap();

        let block2 = BillBlock::new(
            bill_id_test(),
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts + 1,
            Sha256Hash::new("ph2"),
        )
        .unwrap();

        let mut chain = BillBlockchain::new_from_blocks(vec![block1, block2]).unwrap();
        chain.truncate_from(BlockId::first());
        assert_eq!(chain.blocks().len(), 0);
    }

    #[test]
    fn test_truncate_from_beyond_end() {
        use crate::protocol::blockchain::Blockchain;
        use crate::protocol::blockchain::bill::chain::BillBlockchain;

        let keys = BcrKeys::new();
        let bill_keys = BcrKeys::new();
        let ts = test_ts();

        let block1 = BillBlock::new(
            bill_id_test(),
            BlockId::first(),
            Sha256Hash::new("genesis"),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts,
            Sha256Hash::new("ph1"),
        )
        .unwrap();

        let block2 = BillBlock::new(
            bill_id_test(),
            BlockId::next_from_previous_block_id(&block1.id),
            block1.hash.clone(),
            Vec::new(),
            BillOpCode::Issue,
            &keys,
            None,
            &bill_keys,
            ts + 1,
            Sha256Hash::new("ph2"),
        )
        .unwrap();

        let mut chain = BillBlockchain::new_from_blocks(vec![block1, block2]).unwrap();
        chain.truncate_from(BlockId::first().add(10));
        assert_eq!(chain.blocks().len(), 2);
    }
}
