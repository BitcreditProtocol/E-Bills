use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::BlockId;
use crate::protocol::ProtocolValidationError;
use crate::protocol::SchnorrSignature;
use crate::protocol::Sha256Hash;
use crate::protocol::Timestamp;
use crate::protocol::crypto;
use borsh::{BorshDeserialize, BorshSerialize, to_vec};
use log::{error, warn};
use std::fmt::Display;

pub mod bill;
pub mod company;
pub mod identity;

/// Generic result type
pub type Result<T> = std::result::Result<T, Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// Errors from io handling, or binary serialization/deserialization
    #[error("io error {0}")]
    Io(#[from] std::io::Error),

    /// If a whole chain is not valid
    #[error("Blockchain is invalid")]
    BlockchainInvalid,

    /// If certain block is not valid and can't be added
    #[error("Block is invalid")]
    BlockInvalid,

    /// If certain block data is not valid
    #[error("Block data is invalid: {0}")]
    BlockDataInvalid(#[from] ProtocolValidationError),

    /// If certain block's signature does not match the signer in the block data
    #[error("Block's signature does not match the signer")]
    BlockSignatureDoesNotMatchSigner,

    /// If a block operation is not valid for an operation
    #[error("Block operation not valid for this operation")]
    InvalidBlockOp,

    /// Errors stemming from cryptography, such as converting keys, encryption and decryption
    #[error("Secp256k1Cryptography error: {0}")]
    Secp256k1Cryptography(#[from] crypto::Error),

    /// Errors stemming from base58 decoding
    #[error("Base 58 Decode error: {0}")]
    Base58Decode(#[from] bitcoin::base58::InvalidCharacterError),

    /// The given blockchain type string could not be converted to a valid type
    #[error("Invalid blockchain type: {0}")]
    InvalidBlockchainType(String),

    /// Errors from JSON serialization
    #[error("JSON serialization error: {0}")]
    JSON(String),
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub enum BlockchainType {
    #[serde(rename = "bill")]
    Bill,
    #[serde(rename = "company")]
    Company,
    #[serde(rename = "identity")]
    Identity,
}

impl TryFrom<&str> for BlockchainType {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        match value {
            "bill" => Ok(BlockchainType::Bill),
            "company" => Ok(BlockchainType::Company),
            "identity" => Ok(BlockchainType::Identity),
            _ => Err(Error::InvalidBlockchainType(value.to_string())),
        }
    }
}

impl Display for BlockchainType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockchainType::Bill => write!(f, "bill"),
            BlockchainType::Company => write!(f, "company"),
            BlockchainType::Identity => write!(f, "identity"),
        }
    }
}

/// Generic trait for a Block within a Blockchain
pub trait Block {
    type OpCode: PartialEq + Clone + BorshSerialize;
    type BlockDataToHash: BorshSerialize;

    fn id(&self) -> BlockId;
    fn timestamp(&self) -> Timestamp;
    fn op_code(&self) -> &Self::OpCode;
    fn plaintext_hash(&self) -> &Sha256Hash;
    fn hash(&self) -> &Sha256Hash;
    fn previous_hash(&self) -> &Sha256Hash;
    fn data(&self) -> &[u8];
    fn signature(&self) -> &SchnorrSignature;
    fn public_key(&self) -> &PublicKey;
    fn validate(&self) -> bool;
    fn get_block_data_to_hash(&self) -> Self::BlockDataToHash;
    fn validate_plaintext_hash(&self, private_key: &bitcoin::secp256k1::SecretKey) -> bool;

    /// Calculates the plaintext hash over the unencrypted data of the block
    fn calculate_plaintext_hash<T: BorshSerialize>(block_data: &T) -> Result<Sha256Hash> {
        let serialized = to_vec(&block_data)?;
        Ok(Sha256Hash::from_bytes(&serialized))
    }

    /// Calculates the hash over the data to hash for this block
    fn calculate_hash(block_data_to_hash: Self::BlockDataToHash) -> Result<Sha256Hash> {
        let serialized = to_vec(&block_data_to_hash)?;
        Ok(Sha256Hash::from_bytes(&serialized))
    }

    /// Validates that the block's hash is correct
    fn validate_hash(&self) -> bool {
        match Self::calculate_hash(self.get_block_data_to_hash()) {
            Err(e) => {
                error!("Error calculating hash: {e}");
                false
            }
            Ok(calculated_hash) => self.hash() == &calculated_hash,
        }
    }

    /// Verifys the block by checking if the signature is correct
    fn verify(&self) -> bool {
        match self.signature().verify(self.hash(), self.public_key()) {
            Err(e) => {
                error!("Error while verifying block id {}: {e}", self.id());
                false
            }
            Ok(res) => res && self.validate(),
        }
    }

    /// Validates the block with a given previous block
    fn validate_with_previous(&self, previous_block: &Self) -> bool {
        if self.previous_hash() != previous_block.hash() {
            warn!("block with id: {} has wrong previous hash", self.id());
            return false;
        } else if self.timestamp() < previous_block.timestamp() {
            warn!(
                "block with id: {} has a timestamp lower than the previous block: {}",
                self.id(),
                previous_block.timestamp()
            );
            return false;
        } else if !self.id().validate_with_previous(&previous_block.id()) {
            warn!(
                "block with id: {} is not the next block after the previous block: {}",
                self.id(),
                previous_block.id()
            );
            return false;
        } else if !self.validate_hash() {
            warn!("block with id: {} has invalid hash", self.id());
            return false;
        } else if !self.verify() {
            warn!("block with id: {} has invalid signature", self.id());
            return false;
        }
        true
    }
}

/// Generic trait for a Blockchain, expects there to always be at least one block after creation
pub trait Blockchain {
    type Block: Block + Clone;

    fn blocks(&self) -> &Vec<Self::Block>;

    fn blocks_mut(&mut self) -> &mut Vec<Self::Block>;

    /// returns the current height of this blockchain
    fn block_height(&self) -> usize {
        self.blocks().len()
    }

    /// Validates the integrity of the blockchain by checking the validity of each block in the chain.
    fn is_chain_valid(&self) -> bool {
        let blocks = self.blocks();
        for i in 0..blocks.len() {
            if i == 0 {
                continue;
            }
            let first = &blocks[i - 1];
            let second = &blocks[i];
            if !second.validate_with_previous(first) {
                return false;
            }
        }
        true
    }

    /// Trys to add a block to the blockchain, checking the block with the current latest block
    ///
    /// # Arguments
    /// * `block` - The `Block` to be added to the list.
    ///
    /// # Returns
    /// * `true` if the block was successfully added to the list.
    /// * `false` if the block was invalid and could not be added.
    ///
    fn try_add_block(&mut self, block: Self::Block) -> bool {
        let latest_block = self.get_latest_block();
        if block.validate_with_previous(latest_block) {
            self.blocks_mut().push(block);
            true
        } else {
            error!("could not add block - invalid");
            false
        }
    }

    /// Retrieves the latest (most recent) block in the blocks list.
    fn get_latest_block(&self) -> &Self::Block {
        self.blocks().last().expect("there is at least one block")
    }

    /// Retrieves the first block in the blocks list.
    fn get_first_block(&self) -> &Self::Block {
        self.blocks().first().expect("there is at least one block")
    }

    /// Returns the blocks that can be safely added from another chain, checking the consistency of
    /// the chain after every block
    fn get_blocks_to_add_from_other_chain(&mut self, other_chain: &Self) -> Vec<Self::Block> {
        let local_chain_last_id = self.get_latest_block().id();
        let other_chain_last_id = other_chain.get_latest_block().id();
        let mut blocks_to_add = vec![];

        // if it's not the same id, and the local chain is shorter
        if let Some(difference_in_id) =
            local_chain_last_id.difference_to_smaller(&other_chain_last_id)
        {
            for block_id in 1..difference_in_id + 1 {
                let block = other_chain.get_block_by_id(&local_chain_last_id.add(block_id));
                let try_add_block = self.try_add_block(block.clone());
                if try_add_block && self.is_chain_valid() {
                    blocks_to_add.push(block);
                    continue;
                } else {
                    break;
                }
            }
        }
        blocks_to_add
    }

    /// Retrieves the last block with the specified op code, or None if the block is not in the
    /// chain
    fn get_last_version_block_with_op_code(
        &self,
        op_code: <Self::Block as Block>::OpCode,
    ) -> Option<&Self::Block> {
        self.blocks()
            .iter()
            .rfind(|block| block.op_code() == &op_code)
    }

    /// Checks if there is any block with a given operation code in the current blocks list.
    fn block_with_operation_code_exists(&self, op_code: <Self::Block as Block>::OpCode) -> bool {
        self.blocks().iter().any(|b| b.op_code() == &op_code)
    }

    /// Gets the block with the given block number, or the first one, if the given one doesn't
    /// exist
    fn get_block_by_id(&self, id: &BlockId) -> Self::Block {
        self.blocks()
            .iter()
            .find(|b| b.id() == *id)
            .cloned()
            .unwrap_or_else(|| self.get_first_block().clone())
    }

    /// Removes all blocks with id >= from_block_id from the in-memory block vector.
    /// Used for fork resolution - truncates the chain at the divergence point.
    fn truncate_from(&mut self, from_block_id: BlockId) {
        self.blocks_mut().retain(|b| b.id() < from_block_id);
    }
}

fn borsh_to_json_value<T: borsh::BorshDeserialize + serde::Serialize>(
    bytes: &[u8],
) -> Result<serde_json::Value> {
    let block_data: T = borsh::from_slice(bytes)?;
    serde_json::to_value(&block_data).map_err(|e| Error::JSON(e.to_string()))
}
