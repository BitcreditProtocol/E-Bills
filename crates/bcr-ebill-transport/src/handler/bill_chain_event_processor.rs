use crate::handler::public_chain_helpers::{
    BlockData, EventContainer, is_fork_block, resolve_event_chains, resolve_fork,
};
use crate::{Error, Result};
use async_trait::async_trait;
use bcr_common::core::BillId;
use bcr_ebill_api::external::mint::MintClientApi;
use bcr_ebill_api::get_config;
use bcr_ebill_api::service::transport_service::transport_client::TransportClientApi;
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::protocol::Validate;
use bcr_ebill_core::protocol::blockchain::bill::BillOpCode;
use bcr_ebill_core::protocol::blockchain::bill::block::{
    BillIssueBlockData, BillOfferToSellBlockData, BillRequestRecourseBlockData,
    BillRequestToPayBlockData,
};
use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
use bcr_ebill_core::protocol::blockchain::bill::{BillBlock, BillBlockchain};
use bcr_ebill_core::protocol::blockchain::bill::{
    BillValidateActionData, BillValidationActionMode,
};
use bcr_ebill_core::protocol::blockchain::{Block, Blockchain, BlockchainType};
use bcr_ebill_core::protocol::crypto::{BcrKeys, btc};
use bcr_ebill_core::protocol::{BitcoinAddress, BlockId, Sha256Hash};
use bcr_ebill_persistence::{
    FileReferenceStoreApi, NostrChainEventStoreApi, bill::BillChainStoreApi, bill::BillStoreApi,
};
use bitcoin::secp256k1::PublicKey;
use log::{debug, error, info, warn};
use std::sync::Arc;

use super::inbound_file_anchor::{anchor_important_file, bill_file_context};
use super::{BillChainEventProcessorApi, NostrContactProcessorApi};

impl ServiceTraitBounds for BillChainEventProcessor {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl BillChainEventProcessorApi for BillChainEventProcessor {
    async fn process_chain_data(
        &self,
        bill_id: &BillId,
        blocks: Vec<BillBlock>,
        keys: Option<BcrKeys>,
    ) -> Result<()> {
        // check that incoming bills are of the same network that we use
        if bill_id.network() != self.bitcoin_network {
            warn!("Received bill blocks for bill {bill_id} for a different network");
            return Err(Error::Blockchain(format!(
                "Received bill blocks for bill {bill_id} for a different network"
            )));
        }

        if let Ok(existing_chain) = self.bill_blockchain_store.get_chain(bill_id).await {
            self.add_bill_blocks(bill_id, existing_chain, blocks, false)
                .await
        } else {
            match keys {
                Some(keys) => self.add_new_chain(blocks, &keys).await,
                _ => {
                    error!("Received bill blocks for unknown bill {bill_id}");
                    Err(Error::Blockchain(
                        "Received bill blocks for unknown bill".to_string(),
                    ))
                }
            }
        }
    }

    async fn validate_chain_event_and_sender(
        &self,
        bill_id: &BillId,
        sender: nostr::key::PublicKey,
    ) -> Result<bool> {
        if let (Ok(bill_keys), Ok(chain)) = (
            self.bill_store.get_keys(bill_id).await,
            self.bill_blockchain_store.get_chain(bill_id).await,
        ) {
            let participants = chain
                .get_all_nodes_from_bill(&bill_keys)
                .map_err(|e| Error::Blockchain(e.to_string()))?
                .iter()
                .map(|p| p.npub())
                .collect::<Vec<nostr::key::PublicKey>>();

            Ok(participants.contains(&sender))
        } else {
            Ok(false)
        }
    }

    async fn resolve_chain(
        &self,
        bill_id: &BillId,
        bill_keys: &BcrKeys,
    ) -> Result<Vec<Vec<EventContainer>>> {
        resolve_event_chains(
            self.transport.clone(),
            &bill_id.to_string(),
            BlockchainType::Bill,
            &Some(bill_keys.to_owned()),
        )
        .await
    }

    async fn resync_chain(&self, bill_id: &BillId, from_nostr: bool) -> Result<()> {
        if !from_nostr {
            debug!("invalidating cache for {bill_id} without Nostr refetch");
            return self.invalidate_cache_for_bill(bill_id).await;
        }

        match (
            self.bill_blockchain_store.get_chain(bill_id).await,
            self.bill_store.get_keys(bill_id).await,
        ) {
            (Ok(mut existing_chain), Ok(bill_keys)) => {
                debug!("starting bill chain resync for {bill_id}");
                let bcr_keys = BcrKeys::from_private_key(&bill_keys.get_private_key());
                // Pre-fetch validation data needed for all chain candidates
                let is_paid = self.bill_store.is_paid(bill_id).await.map_err(|e| {
                    error!("Could not resync bill {bill_id} because getting paid status failed");
                    Error::Persistence(e.to_string())
                })?;
                let bill_first_version = existing_chain
                    .get_first_version_bill(&bill_keys)
                    .map_err(|e| {
                        error!("Could not resync bill {bill_id} because getting first version bill data failed");
                        Error::Blockchain(e.to_string())
                    })?;

                if let Ok(chain_data) = resolve_event_chains(
                    self.transport.clone(),
                    &bill_id.to_string(),
                    BlockchainType::Bill,
                    &Some(bcr_keys.to_owned()),
                )
                .await
                {
                    for data in chain_data.iter() {
                        let blocks: Vec<BillBlock> = data
                            .iter()
                            .filter_map(|d| match d.block.clone() {
                                BlockData::Bill(block) => Some(block),
                                _ => None,
                            })
                            .collect();

                        if blocks.is_empty() {
                            continue;
                        }

                        let (is_preferred, fork_point) =
                            resolve_fork(existing_chain.blocks(), &blocks);

                        if !is_preferred {
                            continue;
                        }

                        let mut test_chain = existing_chain.clone();
                        if let Some(fork_id) = &fork_point {
                            test_chain.truncate_from(*fork_id);
                        }

                        match self
                            .validate_blocks_for_chain(
                                bill_id,
                                &mut test_chain,
                                &blocks,
                                &bill_keys,
                                &bill_first_version,
                                is_paid,
                            )
                            .await
                        {
                            Ok(()) => {
                                if let Some(fork_id) = fork_point {
                                    info!(
                                        "Fork resolution for bill {bill_id}: replacing blocks from height {fork_id} with preferred remote chain"
                                    );
                                    if let Err(e) = self
                                        .bill_blockchain_store
                                        .remove_blocks_from_height(bill_id, fork_id)
                                        .await
                                    {
                                        error!(
                                            "Failed to remove blocks from height for bill {bill_id}: {e}"
                                        );
                                        return Err(Error::Persistence(
                                            "Failed to remove blocks from height for fork resolution"
                                                .to_string(),
                                        ));
                                    }
                                    existing_chain.truncate_from(fork_id);
                                }

                                if let Err(e) = self
                                    .add_bill_blocks(bill_id, existing_chain, blocks, true)
                                    .await
                                {
                                    error!(
                                        "Failed to add blocks after truncation for bill {bill_id}: {e}"
                                    );
                                    return Err(e);
                                }

                                if let Err(e) = self
                                    .chain_event_store
                                    .remove_chain_events(&bill_id.to_string(), BlockchainType::Bill)
                                    .await
                                {
                                    error!(
                                        "Failed to invalidate old bill chain events during resync: {e}"
                                    );
                                }

                                for event_container in data.iter() {
                                    let event = event_container.as_chain_store_event(
                                        &bill_id.to_string(),
                                        BlockchainType::Bill,
                                        event_container.block.get_block_height() as usize,
                                    );
                                    if let Err(e) =
                                        self.chain_event_store.add_chain_event(event).await
                                    {
                                        error!(
                                            "Failed to store bill chain event during resync: {e}"
                                        );
                                    }
                                }

                                debug!("resynced bill {bill_id} with {} remote events", data.len());
                                return Ok(());
                            }
                            Err(e) => {
                                debug!("Chain candidate failed validation for bill {bill_id}: {e}");
                                continue;
                            }
                        }
                    }

                    debug!("finished bill chain resync for {bill_id}");
                    Ok(())
                } else {
                    let message = format!("Could not refetch chain data from Nostr for {bill_id}");
                    error!("{message}");
                    Err(Error::Network(message))
                }
            }
            _ => {
                let message = format!(
                    "Could not refetch chain for {bill_id} because the bill keys or chain could not be fetched"
                );
                error!("{message}");
                Err(Error::Persistence(message))
            }
        }
    }

    async fn invalidate_cache_for_bill(&self, bill_id: &BillId) -> Result<()> {
        if let Err(e) = self.bill_store.invalidate_bill_in_cache(bill_id).await {
            error!("Failed to invalidate cache for bill {bill_id}: {e}");
            return Err(Error::Persistence(
                "Failed to invalidate cache for bill".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct BillChainEventProcessor {
    bill_blockchain_store: Arc<dyn BillChainStoreApi>,
    bill_store: Arc<dyn BillStoreApi>,
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
    nostr_contact_processor: Arc<dyn NostrContactProcessorApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    transport: Arc<dyn TransportClientApi>,
    mint_client: Arc<dyn MintClientApi>,
    bitcoin_network: bitcoin::Network,
}

impl BillChainEventProcessor {
    pub fn new(
        bill_blockchain_store: Arc<dyn BillChainStoreApi>,
        bill_store: Arc<dyn BillStoreApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        nostr_contact_processor: Arc<dyn NostrContactProcessorApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
        transport: Arc<dyn TransportClientApi>,
        mint_client: Arc<dyn MintClientApi>,
        bitcoin_network: bitcoin::Network,
    ) -> Self {
        Self {
            bill_blockchain_store,
            bill_store,
            file_reference_store,
            nostr_contact_processor,
            chain_event_store,
            transport,
            mint_client,
            bitcoin_network,
        }
    }

    async fn add_bill_blocks(
        &self,
        bill_id: &BillId,
        existing: BillBlockchain,
        blocks: Vec<BillBlock>,
        from_resync: bool,
    ) -> Result<()> {
        let mut block_added = false;
        let mut chain = existing;
        let bill_keys = self.bill_store.get_keys(bill_id).await.map_err(|e| {
            error!("Could not process received blocks for {bill_id} because the bill keys could not be fetched");
            Error::Persistence(e.to_string())
        })?;
        let is_paid = self.bill_store.is_paid(bill_id).await.map_err(|e| {
            error!("Could not process received blocks for bill {bill_id} because getting paid status failed");
            Error::Persistence(e.to_string())
        })?;
        let bill_first_version = chain.get_first_version_bill(&bill_keys).map_err(|e| {
            error!("Could not process received blocks for bill {bill_id} because getting first version bill data failed");
            Error::Blockchain(e.to_string())
        })?;

        for file in &bill_first_version.files {
            anchor_important_file(
                &self.file_reference_store,
                file,
                bill_file_context(bill_id, "files"),
            )
            .await?;
        }

        debug!("adding {} bill blocks for bill {bill_id}", blocks.len());
        let mut block_height = chain.get_latest_block().id;
        for block in blocks.iter() {
            if block.id <= block_height {
                if blocks.len() == 1 && !from_resync {
                    let latest = chain.get_latest_block();
                    if is_fork_block(latest, block) {
                        info!(
                            "Split chain detected for bill {bill_id} at height {} - resyncing",
                            block.id
                        );
                        self.resync_chain(bill_id, true).await?;
                        return Ok(());
                    }
                }
                info!(
                    "Skipping bill block with id {block_height} for {bill_id} as we already have it"
                );
                continue;
            }
            block_added = match self
                .validate_and_save_block(
                    bill_id,
                    &mut chain,
                    &bill_first_version,
                    &bill_keys,
                    block.clone(),
                    is_paid,
                )
                .await
            {
                Ok(added) => {
                    block_height = block.id;
                    Ok(added)
                }
                Err(e) => {
                    // if we received a single block (normal block populate) and we are missing blocks, we try to resync
                    if blocks.len() == 1
                        && !from_resync
                        && BlockId::next_from_previous_block_id(&chain.get_latest_block().id)
                            < block.id
                    {
                        info!(
                            "Received invalid block {} for bill {bill_id} - missing blocks - try to resync",
                            block.id
                        );
                        self.resync_chain(bill_id, true).await?;
                        break;
                    } else {
                        error!("Error adding block for bill {bill_id}: {e}");
                        Err(e)
                    }
                }
            }?;
        }
        // if the bill was changed, we invalidate the cache
        if block_added {
            debug!("block was added for bill {bill_id} - invalidating cache");
            if let Err(e) = self.invalidate_cache_for_bill(bill_id).await {
                error!("Error invalidating cache for bill {bill_id}: {e}");
            }

            // ensure that we have all nostr contacts for the bill participants
            let node_ids = chain
                .get_all_nodes_from_bill(&bill_keys)
                .unwrap_or_default();
            for node_id in node_ids {
                self.nostr_contact_processor
                    .ensure_nostr_contact(&node_id)
                    .await
            }
        }
        Ok(())
    }

    /// Validates a single block for a chain.
    /// Returns true if the block was validated and added to the in-memory chain.
    /// Returns false if the block should be skipped (already exists, etc.).
    async fn validate_block_for_chain(
        &self,
        bill_id: &BillId,
        chain: &mut BillBlockchain,
        bill_first_version: &BillIssueBlockData,
        bill_keys: &BcrKeys,
        block: &BillBlock,
        is_paid: bool,
    ) -> Result<bool> {
        let block_height = chain.get_latest_block().id;
        let block_id = block.id;

        // if we already have the block, we skip it
        if block.id <= block_height {
            info!("Skipping block with id {block_id} for {bill_id} as we already have it");
            return Ok(false);
        }

        if block.op_code == BillOpCode::Issue {
            info!(
                "Skipping block {block_id} with op code Issue for {bill_id} as we already have the chain"
            );
            return Ok(false);
        }

        // validate plaintext hash
        if !block.validate_plaintext_hash(&bill_keys.get_private_key()) {
            error!("Received invalid block {block_id} for bill {bill_id} - invalid plaintext hash");
            return Err(Error::Blockchain(format!(
                "Received invalid block {block_id} for bill {bill_id} - invalid plaintext hash"
            )));
        }

        // create a clone of the chain for validating the bill action later, since the chain
        // will be mutated with the integrity checks
        let chain_clone_for_validation = chain.clone();
        let holder_is_mint_for_validation = match chain_clone_for_validation
            .holder_is_mint(bill_keys)
        {
            Ok(h) => h,
            Err(e) => {
                error!(
                    "Received invalid block {block_id} for bill {bill_id} - could not check if holder is mint for bill {bill_id}"
                );
                return Err(Error::Blockchain(e.to_string()));
            }
        };
        let latest_block_before_add = chain_clone_for_validation.get_latest_block();

        // first, do cheap integrity checks (mutates chain in-memory)
        if !chain.try_add_block(block.clone()) {
            error!("Received invalid block {block_id} for bill {bill_id}");
            return Err(Error::Blockchain(
                "Received invalid block for bill".to_string(),
            ));
        }

        // then, verify signature and signer of the block and get signer and bill action for
        // the block
        let (signer, bill_action) = match block.verify_and_get_signer(bill_keys) {
            Ok(signer) => signer,
            Err(e) => {
                error!(
                    "Received invalid block {block_id} for bill {bill_id} - could not verify signature from block data signer"
                );
                return Err(Error::Blockchain(e.to_string()));
            }
        };

        // validate payment address for payment requests
        match block.op_code() {
            BillOpCode::RequestToPay => {
                let data: BillRequestToPayBlockData = block
                    .get_decrypted_block(bill_keys)
                    .map_err(|e| Error::Blockchain(e.to_string()))?;
                self.validate_payment_address_for_payment_request(
                    bill_id,
                    block.op_code().to_owned(),
                    &block.id(),
                    latest_block_before_add.hash(),
                    &bill_keys.pub_key(),
                    &signer.pub_key(),
                    get_config().bitcoin_network(),
                    &data.payment_data.payment_address,
                    &holder_is_mint_for_validation,
                )
                .await
                .map_err(|e| Error::Blockchain(e.to_string()))?;
            }
            BillOpCode::OfferToSell => {
                let data: BillOfferToSellBlockData = block
                    .get_decrypted_block(bill_keys)
                    .map_err(|e| Error::Blockchain(e.to_string()))?;
                self.validate_payment_address_for_payment_request(
                    bill_id,
                    block.op_code().to_owned(),
                    &block.id(),
                    latest_block_before_add.hash(),
                    &bill_keys.pub_key(),
                    &signer.pub_key(),
                    get_config().bitcoin_network(),
                    &data.payment_data.payment_address,
                    &None,
                )
                .await
                .map_err(|e| Error::Blockchain(e.to_string()))?;
            }
            BillOpCode::RequestRecourse => {
                let data: BillRequestRecourseBlockData = block
                    .get_decrypted_block(bill_keys)
                    .map_err(|e| Error::Blockchain(e.to_string()))?;
                self.validate_payment_address_for_payment_request(
                    bill_id,
                    block.op_code().to_owned(),
                    &block.id(),
                    latest_block_before_add.hash(),
                    &bill_keys.pub_key(),
                    &signer.pub_key(),
                    get_config().bitcoin_network(),
                    &data.payment_data.payment_address,
                    &None,
                )
                .await
                .map_err(|e| Error::Blockchain(e.to_string()))?;
            }
            _ => {} // nothing to do for non-payment-requests
        };

        // then, validate the bill action
        let bill_parties = chain_clone_for_validation
            .get_bill_parties(bill_keys, bill_first_version)
            .map_err(|e| {
                error!("Received invalid block {block_id} for bill {bill_id}: {e}");
                Error::Blockchain(
                    "Received invalid block for bill - couldn't get bill parties".to_string(),
                )
            })?;

        if let Err(e) = (BillValidateActionData {
            blockchain: chain_clone_for_validation,
            drawee_node_id: bill_parties.drawee.node_id,
            payee_node_id: bill_parties.payee.node_id(),
            endorsee_node_id: bill_parties.endorsee.map(|e| e.node_id()),
            maturity_date: bill_first_version.maturity_date.clone(),
            bill_keys: bill_keys.clone(),
            timestamp: block.timestamp,
            signer_node_id: signer,
            is_paid,
            mode: BillValidationActionMode::Deep(
                bill_action.ok_or_else(|| {
                    error!(
                        "Received invalid block {block_id} for bill {bill_id} - no valid bill action returned"
                    );
                    Error::Blockchain(
                        "Received invalid block for bill - no valid bill action returned"
                            .to_string(),
                    )
                })?),
        })
        .validate()
        {
            error!(
                "Received invalid block {block_id} for bill {bill_id}, bill action validation failed: {e}"
            );
            return Err(Error::Blockchain(e.to_string()));
        }

        Ok(true) // block was validated and added to in-memory chain
    }

    /// Calculates the payment address with the given values and validates it against the given address
    async fn validate_payment_address_for_payment_request(
        &self,
        bill_id: &BillId,
        payment_op: BillOpCode,
        block_id: &BlockId,
        previous_hash: &Sha256Hash,
        bill_pub_key: &PublicKey,
        caller_pub_key: &PublicKey,
        btc_network: bitcoin::Network,
        address_to_check: &BitcoinAddress,
        holder_is_mint: &Option<BillParticipant>,
    ) -> Result<()> {
        if let Some(_mint_holder) = holder_is_mint
            && matches!(payment_op, BillOpCode::RequestToPay)
        {
            if !address_to_check.is_valid_for_network(btc_network) {
                return Err(Error::Crypto(
                    "Wrong Network for mint payment address".to_owned(),
                ));
            }

            self.mint_client
                .validate_payment_address_from_mint(
                    &get_config().mint_config.default_mint_url,
                    address_to_check,
                    bill_id,
                    *block_id,
                    previous_hash,
                )
                .await
                .map_err(|e| {
                    error!("Invalid mint payment address: {e}");
                    Error::Crypto("Invalid mint payment address".to_owned())
                })?;
            Ok(())
        } else {
            let addr = btc::get_address_to_pay(
                bill_pub_key,
                caller_pub_key,
                &btc::calculate_tweak_hash_for_payment_request(
                    payment_op,
                    block_id,
                    previous_hash,
                )?,
                btc_network,
            )?;
            if &addr != address_to_check {
                Err(Error::Crypto(format!(
                    "Payment Addresses don't match - derived: {}, to check: {}",
                    addr.assume_checked(),
                    address_to_check.clone().assume_checked(),
                )))
            } else {
                Ok(())
            }
        }
    }

    /// Validates multiple blocks for a chain WITHOUT persisting.
    /// Returns Ok(()) if all blocks are valid, Err otherwise.
    async fn validate_blocks_for_chain(
        &self,
        bill_id: &BillId,
        chain: &mut BillBlockchain,
        blocks: &[BillBlock],
        bill_keys: &BcrKeys,
        bill_first_version: &BillIssueBlockData,
        is_paid: bool,
    ) -> Result<()> {
        for block in blocks {
            self.validate_block_for_chain(
                bill_id,
                chain,
                bill_first_version,
                bill_keys,
                block,
                is_paid,
            )
            .await?;
        }
        Ok(())
    }

    /// Validates and saves a single block to the chain.
    /// Returns true if the block was added, false if it was skipped.
    async fn validate_and_save_block(
        &self,
        bill_id: &BillId,
        chain: &mut BillBlockchain,
        bill_first_version: &BillIssueBlockData,
        bill_keys: &BcrKeys,
        block: BillBlock,
        is_paid: bool,
    ) -> Result<bool> {
        let added = self
            .validate_block_for_chain(
                bill_id,
                chain,
                bill_first_version,
                bill_keys,
                &block,
                is_paid,
            )
            .await?;

        if added {
            // if everything works out - add the block
            self.save_block(bill_id, &block).await?;
        }

        Ok(added)
    }

    async fn add_new_chain(&self, blocks: Vec<BillBlock>, keys: &BcrKeys) -> Result<()> {
        let (bill_id, bill_first_version, chain) = self.get_valid_chain(blocks, keys)?;
        debug!("adding new chain for bill {bill_id}");
        // issue block was validated in get_valid_chain
        let issue_block = chain.get_first_block().to_owned();
        // validate plaintext hash
        if !issue_block.validate_plaintext_hash(&keys.get_private_key()) {
            error!("Newly received chain issue block has invalid plaintext hash");
            return Err(Error::Blockchain(
                "Newly received chain issue block has invalid plaintext hash".to_string(),
            ));
        }
        // create a chain that starts from issue, to simulate adding blocks and validating them
        let mut chain_starting_at_issue =
            match BillBlockchain::new_from_blocks(vec![issue_block.clone()]) {
                Ok(chain) => chain,
                Err(e) => {
                    error!("Newly received chain is not valid: {e}");
                    return Err(Error::Blockchain(
                        "Newly received chain is not valid".to_string(),
                    ));
                }
            };
        self.save_block(&bill_id, &issue_block).await?;

        for file in &bill_first_version.files {
            anchor_important_file(
                &self.file_reference_store,
                file,
                bill_file_context(&bill_id, "files"),
            )
            .await?;
        }

        // Only add other blocks, if there are any
        if chain.block_height() > 1 {
            let blocks = chain.blocks()[1..].to_vec();
            for block in blocks {
                self.validate_and_save_block(
                    &bill_id,
                    &mut chain_starting_at_issue,
                    &bill_first_version,
                    keys,
                    block,
                    false, // new chain, we don't know if it's paid
                )
                .await?;
            }
        }
        self.save_keys(&bill_id, keys).await?;

        // ensure that we have all nostr contacts for the bill participants
        let node_ids = chain.get_all_nodes_from_bill(keys).unwrap_or_default();
        for node_id in node_ids {
            self.nostr_contact_processor
                .ensure_nostr_contact(&node_id)
                .await
        }

        Ok(())
    }

    fn get_valid_chain(
        &self,
        blocks: Vec<BillBlock>,
        keys: &BcrKeys,
    ) -> Result<(BillId, BillIssueBlockData, BillBlockchain)> {
        // cheap integrity checks first
        match BillBlockchain::new_from_blocks(blocks) {
            Ok(chain) if chain.is_chain_valid() => {
                // make sure first block is of type Issue
                if chain.get_first_block().op_code != BillOpCode::Issue {
                    error!("Newly received chain is not valid - first block is not an Issue block");
                    return Err(Error::Blockchain(
                        "Newly received chain is not valid - first block is not an Issue block"
                            .to_string(),
                    ));
                }
                match chain.get_first_version_bill(keys) {
                    Ok(bill) => {
                        // then, verify signature and signer of each block and get signer
                        for block in chain.blocks().iter() {
                            let _signer = match block.verify_and_get_signer(keys) {
                                Ok(signer) => signer,
                                Err(e) => {
                                    error!(
                                        "Received invalid block for bill {} - could not verify signature from block data signer",
                                        bill.id
                                    );
                                    return Err(Error::Blockchain(e.to_string()));
                                }
                            };
                        }
                        Ok((bill.id.clone(), bill, chain))
                    }
                    Err(e) => {
                        error!("Failed to get first version bill from newly received chain: {e}");
                        Err(Error::Crypto(format!(
                            "Failed to decrypt new bill chain with given keys: {e}"
                        )))
                    }
                }
            }
            _ => {
                error!("Newly received chain is not valid");
                Err(Error::Blockchain(
                    "Newly received chain is not valid".to_string(),
                ))
            }
        }
    }

    async fn save_block(&self, bill_id: &BillId, block: &BillBlock) -> Result<()> {
        if let Err(e) = self.bill_blockchain_store.add_block(bill_id, block).await {
            error!("Failed to add block to blockchain store: {e}");
            return Err(Error::Persistence(
                "Failed to add block to blockchain store".to_string(),
            ));
        }
        Ok(())
    }

    async fn save_keys(&self, bill_id: &BillId, keys: &BcrKeys) -> Result<()> {
        if let Err(e) = self.bill_store.save_keys(bill_id, keys).await {
            error!("Failed to save keys to bill store: {e}");
            return Err(Error::Persistence(
                "Failed to save keys to bill store".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bcr_common::core::NodeId;
    use bcr_ebill_core::protocol::{
        File, Name, Sha256Hash, Timestamp,
        blockchain::bill::{
            block::{
                BillAcceptBlockData, BillEndorseBlockData, BillParticipantBlockData,
                BillRejectBlockData, BillRequestToAcceptBlockData,
            },
            participant::BillIdentParticipant,
        },
        constants::ACCEPT_DEADLINE_SECONDS,
        crypto::BcrKeys,
        event::{BillBlockEvent, Event, EventEnvelope},
        file_reference::{FileReference, FileReferenceContext},
    };
    use bitcoin::PrivateKey;
    use bitcoin::secp256k1::{SECP256K1, SecretKey};
    use mockall::predicate::{always, eq};
    use nostr::event::FinalizeEvent;
    use std::str::FromStr;

    use crate::{
        handler::{
            MockNostrContactProcessorApi,
            test_utils::{
                MockBillChainStore, MockBillStore, MockMintClient, MockNostrChainEventStore,
                bill_id_test, empty_address, get_baseline_bill, get_baseline_identity,
                get_bill_keys, get_genesis_chain, get_test_bitcredit_bill, node_id_test_other,
                private_key_test,
            },
        },
        test_utils::{
            MockFileReferenceStore, MockNotificationJsonTransport, init_test_cfg,
            signed_identity_proof_test, valid_payment_address_testnet,
        },
        transport::create_public_chain_event,
    };

    use super::*;

    #[tokio::test]
    async fn test_create_event_handler() {
        let (bill_chain_store, bill_store, contact, transport, chain_event_store) = create_mocks();
        BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );
    }

    #[tokio::test]
    async fn test_validate_chain_event_and_sender_invalid_on_no_keys_or_chain() {
        let keys = BcrKeys::new().get_nostr_keys();
        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        bill_store
            .expect_get_keys()
            .with(eq(bill_id_test()))
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });

        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let valid = handler
            .validate_chain_event_and_sender(&bill_id_test(), keys.public_key())
            .await
            .expect("Event should be handled");
        assert!(!valid);
    }

    #[tokio::test]
    async fn test_validate_chain_event_and_sender_valid() {
        let baseline = get_baseline_identity();
        let npub = baseline.key_pair.get_nostr_keys().public_key();
        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        let payer = BillIdentParticipant::new(baseline.identity.clone()).unwrap();
        let payee = BillIdentParticipant::new(baseline.identity.clone()).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);

        bill_store
            .expect_get_keys()
            .with(eq(bill_id_test()))
            .returning(move |_| Ok(get_bill_keys()));

        let chain = get_genesis_chain(Some(bill.clone()));
        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .returning(move |_| Ok(chain.clone()));

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let valid = handler
            .validate_chain_event_and_sender(&bill_id_test(), npub)
            .await
            .expect("Event should be handled");

        assert!(valid);
    }

    #[tokio::test]
    async fn test_creates_new_chain_for_new_chain_event() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let mut payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        payee.node_id = node_id_test_other();
        let drawer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, Some(&drawer), None);
        let chain = get_genesis_chain(Some(bill.clone()));
        let keys = get_bill_keys();

        let (mut bill_chain_store, mut bill_store, mut contact, transport, _) = create_mocks();

        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id_test()), eq(chain.blocks()[0].clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        bill_store
            .expect_save_keys()
            .with(eq(bill_id_test()), always())
            .times(1)
            .returning(move |_, _| Ok(()));

        // Store new contact
        contact.expect_ensure_nostr_contact().returning(|_| ());

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&bill_id_test(), chain.blocks().clone(), Some(keys.clone()))
            .await
            .expect("Event should be handled");
    }

    #[tokio::test]
    async fn test_adds_block_for_existing_chain_event() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let mut endorsee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        endorsee.node_id = node_id_test_other();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));
        let block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            chain.get_latest_block(),
            &BillEndorseBlockData {
                endorsee: BillParticipantBlockData::Ident(endorsee.clone().into()),
                // endorsed by payee
                endorser: BillParticipantBlockData::Ident(
                    BillIdentParticipant::new(get_baseline_identity().identity)
                        .unwrap()
                        .into(),
                ),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, mut bill_store, mut contact, transport, _) = create_mocks();

        let chain_clone = chain.clone();
        bill_store
            .expect_invalidate_bill_in_cache()
            .returning(|_| Ok(()));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_store
            .expect_get_keys()
            .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));
        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| Ok(chain_clone.clone()));

        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id_test()), eq(block.clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        contact.expect_ensure_nostr_contact().returning(|_| ());

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&bill_id_test(), vec![block.clone()], None)
            .await
            .expect("Event should be handled");
    }

    #[tokio::test]
    async fn test_recovers_chain_on_missing_blocks() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let mut endorsee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        endorsee.node_id = node_id_test_other();
        let bcr_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_id = BillId::new(bcr_keys.pub_key(), bitcoin::Network::Testnet);
        let bill = get_test_bitcredit_bill(&bill_id, &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let ts = chain.get_latest_block().timestamp + 1000;
        let block1 = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let block2 = BillBlock::create_block_for_accept(
            bill_id.clone(),
            &block1,
            &BillAcceptBlockData {
                accepter: payer.clone().into(),
                signatory: None,
                signing_timestamp: block1.timestamp + 1000,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &bcr_keys,
            None,
            &bcr_keys,
            block1.timestamp + 1000,
        )
        .unwrap();

        let event1 = generate_test_event(
            &bcr_keys,
            None,
            None,
            as_event_payload(&bill_id_test(), chain.get_latest_block()),
            &bill_id,
        );

        let event2 = generate_test_event(
            &bcr_keys,
            Some(event1.clone()),
            Some(event1.clone()),
            as_event_payload(&bill_id_test(), &block1),
            &bill_id,
        );

        let event3 = generate_test_event(
            &bcr_keys,
            Some(event2.clone()),
            Some(event1.clone()),
            as_event_payload(&bill_id_test(), &block2),
            &bill_id,
        );

        // mock the nostr chain
        let nostr_chain = vec![event1.clone(), event2.clone(), event3.clone()];

        let (
            mut bill_chain_store,
            mut bill_store,
            mut contact,
            mut transport,
            mut chain_event_store,
        ) = create_mocks();

        let chain_clone = chain.clone();
        bill_store
            .expect_invalidate_bill_in_cache()
            .returning(|_| Ok(()));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_store
            .expect_get_keys()
            .returning(move |_| Ok(BcrKeys::from_private_key(&private_key_test())));

        let expected_bill_id = bill_id.clone();
        // on chain recovery we ask for the existing chain two times
        bill_chain_store
            .expect_get_chain()
            .with(eq(expected_bill_id))
            .times(2)
            .returning(move |_| Ok(chain_clone.clone()));

        transport
            .expect_resolve_public_chain()
            .with(eq(bill_id.to_string()), eq(BlockchainType::Bill))
            .returning(move |_, _| Ok(nostr_chain.clone()))
            .once();

        // recovery adds the missing block
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id.clone()), eq(block1.clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        // and then the received block
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id.clone()), eq(block2.clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        contact.expect_ensure_nostr_contact().returning(|_| ());

        chain_event_store
            .expect_remove_chain_events()
            .returning(|_, _| Ok(()));

        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(0..);

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&bill_id, vec![block2.clone()], None)
            .await
            .expect("Event should be handled");
    }

    fn as_event_payload(id: &BillId, block: &BillBlock) -> EventEnvelope {
        Event::new_bill(BillBlockEvent {
            bill_id: id.clone(),
            block_height: block.id.inner() as usize,
            block: block.clone(),
        })
        .try_into()
        .expect("could not create envelope")
    }

    fn generate_test_event(
        keys: &BcrKeys,
        previous: Option<nostr::event::Event>,
        root: Option<nostr::event::Event>,
        data: EventEnvelope,
        bill_id: &BillId,
    ) -> nostr::event::Event {
        create_public_chain_event(
            &bill_id.to_string(),
            data,
            Timestamp::new(1000).unwrap(),
            BlockchainType::Bill,
            previous,
            root,
        )
        .expect("could not create chain event")
        .finalize(&keys.get_nostr_keys())
        .expect("could not sign event")
    }

    #[tokio::test]
    async fn test_fails_to_create_new_chain_for_new_chain_event_if_block_validation_fails() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let mut payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        payee.node_id = node_id_test_other();
        let drawer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, Some(&drawer), None);
        let mut chain = get_genesis_chain(Some(bill.clone()));
        let keys = get_bill_keys();

        // reject to pay without a request to accept will fail
        let block = BillBlock::create_block_for_reject_to_pay(
            bill_id_test(),
            chain.get_latest_block(),
            &BillRejectBlockData {
                rejecter: payer.clone().into(),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();
        assert!(chain.try_add_block(block));

        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });
        // should persist the issue block, but fail the second block
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id_test()), eq(chain.blocks()[0].clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        bill_store
            .expect_save_keys()
            .with(eq(bill_id_test()), always())
            .never();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler
            .process_chain_data(&bill_id_test(), chain.blocks().clone(), Some(keys.clone()))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fails_to_create_new_chain_for_new_chain_event_if_block_signing_check_fails() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        // drawer has a different key than signer, signing check will fail
        let mut drawer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        drawer.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, Some(&drawer), None);
        let chain = get_genesis_chain(Some(bill.clone()));
        let keys = get_bill_keys();

        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id_test()), eq(chain.blocks()[0].clone()))
            .never();

        bill_store
            .expect_save_keys()
            .with(eq(bill_id_test()), always())
            .never();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler
            .process_chain_data(&bill_id_test(), chain.blocks().clone(), Some(keys.clone()))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fails_to_add_block_for_invalid_bill_action() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        // reject to pay without a request to accept will fail
        let block = BillBlock::create_block_for_reject_to_pay(
            bill_id_test(),
            chain.get_latest_block(),
            &BillRejectBlockData {
                rejecter: payer.clone().into(),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        let chain_clone = chain.clone();
        bill_store
            .expect_get_keys()
            .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));

        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| Ok(chain_clone.clone()));

        // block is not added
        bill_chain_store.expect_add_block().never();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler
            .process_chain_data(&bill_id_test(), vec![block.clone()], None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fails_to_add_block_for_invalidly_signed_blocks() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let endorsee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        // endorser is different than block signer - signature won't be able to be validated
        let mut endorser = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        endorser.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            chain.get_latest_block(),
            &BillEndorseBlockData {
                endorsee: BillParticipantBlockData::Ident(endorsee.clone().into()),
                // endorsed by payee
                endorser: BillParticipantBlockData::Ident(endorser.clone().into()),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, mut bill_store, contact, transport, _) = create_mocks();

        let chain_clone = chain.clone();
        bill_store
            .expect_get_keys()
            .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));

        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| Ok(chain_clone.clone()));

        // block is not added
        bill_chain_store.expect_add_block().never();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler
            .process_chain_data(&bill_id_test(), vec![block.clone()], None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fails_to_add_block_for_unknown_chain() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let endorsee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let block = BillBlock::create_block_for_endorse(
            bill_id_test(),
            chain.get_latest_block(),
            &BillEndorseBlockData {
                endorsee: BillParticipantBlockData::Ident(endorsee.clone().into()),
                // endorsed by payee
                endorser: BillParticipantBlockData::Ident(
                    BillIdentParticipant::new(get_baseline_identity().identity)
                        .unwrap()
                        .into(),
                ),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, bill_store, contact, transport, _) = create_mocks();

        bill_chain_store
            .expect_get_chain()
            .with(eq(bill_id_test()))
            .times(1)
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "bill block".to_string(),
                    bill_id_test().to_string(),
                ))
            });

        bill_chain_store.expect_add_block().never();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler
            .process_chain_data(&bill_id_test(), vec![block.clone()], None)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_fails_to_add_block_for_bill_id_from_different_network() {
        let (bill_chain_store, bill_store, contact, transport, _) = create_mocks();
        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );
        let mainnet_bill_id = BillId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);

        let result = handler
            .process_chain_data(&mainnet_bill_id, vec![], None)
            .await;

        assert!(result.is_err());
        assert!(result.as_ref().unwrap_err().to_string().contains("network"));
    }

    #[tokio::test]
    async fn test_from_resync_prevents_recursive_resync_on_split_chain() {
        // Setup: Create a scenario where a single block would trigger split chain detection
        // When from_resync=false, it would trigger resync_chain
        // When from_resync=true, it should NOT trigger resync_chain (preventing loop)
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        // Create a valid block first
        let mut split_block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: chain.get_latest_block().timestamp
                    + 1000
                    + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        // Manually modify the block to simulate split chain conditions:
        // Same height as latest, different hash, earlier timestamp
        split_block.id = chain.get_latest_block().id; // Same height
        split_block.timestamp = chain.get_latest_block().timestamp - 1000; // Earlier timestamp
        split_block.hash = Sha256Hash::new("different_hash"); // Different hash

        let (mut bill_chain_store, mut bill_store, contact, mut transport, _) = create_mocks();

        // Setup existing chain
        let chain_clone = chain.clone();
        bill_store
            .expect_get_keys()
            .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_chain_store
            .expect_get_chain()
            .returning(move |_| Ok(chain_clone.clone()));

        // CRITICAL: transport.resolve_public_chain should NEVER be called when from_resync=true
        // This would indicate a resync is being triggered
        transport
            .expect_resolve_public_chain()
            .times(0)
            .returning(|_, _| Ok(vec![]));

        // Block should NOT be added (it's rejected due to split detection, but no resync)
        bill_chain_store.expect_add_block().times(0);

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        // Call add_bill_blocks with from_resync=true
        // Should return Ok without calling resync_chain even though conditions would normally trigger it
        let result = handler
            .add_bill_blocks(&bill_id_test(), chain, vec![split_block], true)
            .await;

        // Should succeed without triggering resync (block is skipped/ignored)
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_from_resync_prevents_recursive_resync_on_missing_blocks() {
        // Setup: Create a scenario where validation fails due to missing predecessor
        // When from_resync=false, it would trigger resync_chain
        // When from_resync=true, it should return error without resync
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        // Create a block at height 2 (we only have block 1, so gap of 1 block)
        // Create an intermediate block first to establish proper chain
        let prev_block = chain.get_latest_block();
        let intermediate_block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            prev_block,
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: prev_block.timestamp + 500,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: prev_block.timestamp
                    + 500
                    + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            prev_block.timestamp + 500,
        )
        .unwrap();

        // Now create a gapped block at height 3, but we'll try to add it without intermediate_block
        // This creates a gap (we have block 1, but trying to add block 3)
        let gapped_block = BillBlock::create_block_for_accept(
            bill_id_test(),
            &intermediate_block,
            &BillAcceptBlockData {
                accepter: payer.clone().into(),
                signatory: None,
                signing_timestamp: intermediate_block.timestamp + 1000,
                signing_address: empty_address(),
                signer_identity_proof: signed_identity_proof_test().into(),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            intermediate_block.timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, mut bill_store, contact, mut transport, _) = create_mocks();

        // Setup existing chain
        let chain_clone = chain.clone();
        bill_store
            .expect_get_keys()
            .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_chain_store
            .expect_get_chain()
            .returning(move |_| Ok(chain_clone.clone()));

        // CRITICAL: transport.resolve_public_chain should NEVER be called when from_resync=true
        transport
            .expect_resolve_public_chain()
            .times(0)
            .returning(|_, _| Ok(vec![]));

        // Block should NOT be added
        bill_chain_store.expect_add_block().times(0);

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        // Call add_bill_blocks with from_resync=true and a gapped block
        // Should return error, NOT call resync_chain
        let result = handler
            .add_bill_blocks(&bill_id_test(), chain, vec![gapped_block], true)
            .await;

        // Should return an error (validation failed), but NOT trigger resync
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_blocks_for_chain_does_not_persist() {
        // Verify that validate_blocks_for_chain does not call add_block
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        // Create valid blocks to validate
        let block1 = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: chain.get_latest_block().timestamp
                    + 1000
                    + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        let (mut bill_chain_store, _, _, _, _) = create_mocks();

        // CRITICAL: add_block should NEVER be called during validation
        bill_chain_store
            .expect_add_block()
            .times(0)
            .returning(|_, _| Ok(()));

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let mut test_chain = chain.clone();
        let bill_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_first_version = chain.get_first_version_bill(&bill_keys).unwrap();

        // Call the pure validation method
        let result = handler
            .validate_blocks_for_chain(
                &bill_id_test(),
                &mut test_chain,
                &[block1],
                &bill_keys,
                &bill_first_version,
                false,
            )
            .await;

        // Should succeed without persisting
        assert!(result.is_ok());
        // Test passes if add_block was never called (mock verifies this)
    }

    #[tokio::test]
    async fn test_validate_blocks_for_chain_fails_on_invalid_block() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        // Create an invalid block (bad signature)
        let mut invalid_block = BillBlock::create_block_for_request_to_accept(
            bill_id_test(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: chain.get_latest_block().timestamp + 1000,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: chain.get_latest_block().timestamp
                    + 1000
                    + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &BcrKeys::from_private_key(&private_key_test()),
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            chain.get_latest_block().timestamp + 1000,
        )
        .unwrap();

        // Tamper with the block to make it invalid
        invalid_block.hash = Sha256Hash::new("tampered_hash");

        let (bill_chain_store, _, _, _, _) = create_mocks();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let mut test_chain = chain.clone();
        let bill_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_first_version = chain.get_first_version_bill(&bill_keys).unwrap();

        // Call the pure validation method with invalid block
        let result = handler
            .validate_blocks_for_chain(
                &bill_id_test(),
                &mut test_chain,
                &[invalid_block],
                &bill_keys,
                &bill_first_version,
                false,
            )
            .await;

        // Should return error without persisting anything
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_resync_chain_early_exit_on_first_valid() {
        // Setup with multiple valid chains - should exit early after first valid one
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let _endorsee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bill = get_test_bitcredit_bill(&bill_id_test(), &payer, &payee, None, None);
        let chain = get_genesis_chain(Some(bill.clone()));

        let bcr_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_id = BillId::new(bcr_keys.pub_key(), bitcoin::Network::Testnet);

        // Create three chains of different lengths
        // Chain 1: 1 block (genesis only - shortest, but valid)
        let ts = chain.get_latest_block().timestamp + 1000;
        let block1 = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            ts,
        )
        .unwrap();

        // Create mock nostr events for three chains (using same events but different ordering)
        let event1 = generate_test_event(
            &bcr_keys,
            None,
            None,
            as_event_payload(&bill_id, chain.get_latest_block()),
            &bill_id,
        );

        let event2 = generate_test_event(
            &bcr_keys,
            Some(event1.clone()),
            Some(event1.clone()),
            as_event_payload(&bill_id, &block1),
            &bill_id,
        );

        // All events (resolve_public_chain returns flat Vec<Event>)
        let all_events = vec![event1.clone(), event2.clone()];

        let (
            mut bill_chain_store,
            mut bill_store,
            mut contact,
            mut transport,
            mut chain_event_store,
        ) = create_mocks();

        let chain_clone = chain.clone();
        bill_store
            .expect_invalidate_bill_in_cache()
            .returning(|_| Ok(()));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_store
            .expect_get_keys()
            .returning(move |_| Ok(BcrKeys::from_private_key(&private_key_test())));

        // on chain recovery we ask for the existing chain
        bill_chain_store
            .expect_get_chain()
            .returning(move |_| Ok(chain_clone.clone()));

        // Mock returning all events (resolve_event_chains will build chains from them)
        transport
            .expect_resolve_public_chain()
            .with(eq(bill_id.to_string()), eq(BlockchainType::Bill))
            .returning(move |_, _| Ok(all_events.clone()))
            .once();

        // block1 should be added exactly once (early exit after first valid chain)
        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id.clone()), eq(block1.clone()))
            .times(1)
            .returning(move |_, _| Ok(()));

        contact.expect_ensure_nostr_contact().returning(|_| ());

        chain_event_store
            .expect_remove_chain_events()
            .returning(|_, _| Ok(()));

        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(0..);

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        // This should process the valid chain and exit
        handler
            .resync_chain(&bill_id, true)
            .await
            .expect("resync should succeed");
    }

    #[tokio::test]
    async fn test_resync_chain_resolves_fork_with_truncation() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bcr_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_id = BillId::new(bcr_keys.pub_key(), bitcoin::Network::Testnet);
        let bill = get_test_bitcredit_bill(&bill_id, &payer, &payee, None, None);

        let genesis_chain = get_genesis_chain(Some(bill.clone()));

        let ts = genesis_chain.get_latest_block().timestamp + 1000;

        let local_block = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            genesis_chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            ts + 2000,
        )
        .unwrap();

        let mut local_chain = genesis_chain.clone();
        local_chain.try_add_block(local_block.clone());

        let remote_block = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            genesis_chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            ts + 1000,
        )
        .unwrap();

        let mut remote_chain = genesis_chain.clone();
        remote_chain.try_add_block(remote_block.clone());

        let fork_height = local_block.id;

        let event0 = generate_test_event(
            &bcr_keys,
            None,
            None,
            as_event_payload(&bill_id, genesis_chain.get_latest_block()),
            &bill_id,
        );

        let event_remote = generate_test_event(
            &bcr_keys,
            Some(event0.clone()),
            Some(event0.clone()),
            as_event_payload(&bill_id, &remote_block),
            &bill_id,
        );

        let nostr_events = vec![event0.clone(), event_remote.clone()];

        let (
            mut bill_chain_store,
            mut bill_store,
            mut contact,
            mut transport,
            mut chain_event_store,
        ) = create_mocks();

        bill_store
            .expect_invalidate_bill_in_cache()
            .returning(|_| Ok(()));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_store
            .expect_get_keys()
            .returning(move |_| Ok(bcr_keys.clone()));

        let local_chain_clone = local_chain.clone();
        bill_chain_store
            .expect_get_chain()
            .returning(move |_| Ok(local_chain_clone.clone()));

        transport
            .expect_resolve_public_chain()
            .with(eq(bill_id.to_string()), eq(BlockchainType::Bill))
            .returning(move |_, _| Ok(nostr_events.clone()))
            .once();

        bill_chain_store
            .expect_remove_blocks_from_height()
            .with(eq(bill_id.clone()), eq(fork_height))
            .times(1)
            .returning(|_, _| Ok(()));

        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id.clone()), eq(remote_block.clone()))
            .times(1)
            .returning(|_, _| Ok(()));

        contact.expect_ensure_nostr_contact().returning(|_| ());

        chain_event_store
            .expect_remove_chain_events()
            .returning(|_, _| Ok(()));

        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(0..);

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        handler
            .resync_chain(&bill_id, true)
            .await
            .expect("resync should succeed with fork resolution");
    }

    #[tokio::test]
    async fn test_resync_chain_skips_non_preferred_chain() {
        let payer = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let payee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
        let bcr_keys = BcrKeys::from_private_key(&private_key_test());
        let bill_id = BillId::new(bcr_keys.pub_key(), bitcoin::Network::Testnet);
        let bill = get_test_bitcredit_bill(&bill_id, &payer, &payee, None, None);

        let genesis_chain = get_genesis_chain(Some(bill.clone()));

        let ts = genesis_chain.get_latest_block().timestamp + 1000;

        let local_block = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            genesis_chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            ts + 1000,
        )
        .unwrap();

        let mut local_chain = genesis_chain.clone();
        local_chain.try_add_block(local_block.clone());

        let remote_block = BillBlock::create_block_for_request_to_accept(
            bill_id.clone(),
            genesis_chain.get_latest_block(),
            &BillRequestToAcceptBlockData {
                requester: BillParticipantBlockData::Ident(payer.clone().into()),
                signatory: None,
                signing_timestamp: ts,
                signing_address: Some(empty_address()),
                signer_identity_proof: Some(signed_identity_proof_test().into()),
                acceptance_deadline_timestamp: ts + 2 * ACCEPT_DEADLINE_SECONDS,
            },
            &bcr_keys,
            None,
            &bcr_keys,
            ts + 2000,
        )
        .unwrap();

        let event0 = generate_test_event(
            &bcr_keys,
            None,
            None,
            as_event_payload(&bill_id, genesis_chain.get_latest_block()),
            &bill_id,
        );

        let event_remote = generate_test_event(
            &bcr_keys,
            Some(event0.clone()),
            Some(event0.clone()),
            as_event_payload(&bill_id, &remote_block),
            &bill_id,
        );

        let nostr_events = vec![event0.clone(), event_remote.clone()];

        let (mut bill_chain_store, mut bill_store, contact, mut transport, mut chain_event_store) =
            create_mocks();

        bill_store
            .expect_invalidate_bill_in_cache()
            .returning(|_| Ok(()));
        bill_store.expect_is_paid().returning(|_| Ok(false));
        bill_store
            .expect_get_keys()
            .returning(move |_| Ok(bcr_keys.clone()));

        let local_chain_clone = local_chain.clone();
        bill_chain_store
            .expect_get_chain()
            .returning(move |_| Ok(local_chain_clone.clone()));

        transport
            .expect_resolve_public_chain()
            .with(eq(bill_id.to_string()), eq(BlockchainType::Bill))
            .returning(move |_, _| Ok(nostr_events.clone()))
            .once();

        bill_chain_store.expect_remove_blocks_from_height().times(0);

        bill_chain_store.expect_add_block().times(0);

        chain_event_store
            .expect_remove_chain_events()
            .returning(|_, _| Ok(()));

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(contact),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler.resync_chain(&bill_id, true).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_add_new_chain_anchors_inbound_bill_files() {
        let mut bill = get_baseline_bill(&bill_id_test());
        let attachment = File {
            name: Name::new("attachment.pdf").unwrap(),
            hash: Sha256Hash::new("bill_attachment_file_hash_1234567890"),
            nostr_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .parse()
                .unwrap(),
        };
        bill.files = vec![attachment.clone()];

        let chain = get_genesis_chain(Some(bill));
        let issue_block = chain.get_first_block().clone();
        let keys = get_bill_keys();

        let mut bill_chain_store = MockBillChainStore::new();
        let mut bill_store = MockBillStore::new();
        let mut file_reference_store = MockFileReferenceStore::new();
        let mut contact = MockNostrContactProcessorApi::new();

        bill_chain_store
            .expect_add_block()
            .with(eq(bill_id_test()), always())
            .returning(|_, _| Ok(()))
            .once();
        bill_store
            .expect_save_keys()
            .returning(|_, _| Ok(()))
            .once();
        contact
            .expect_ensure_nostr_contact()
            .returning(|_| ())
            .times(2);

        file_reference_store
            .expect_upsert()
            .withf(
                move |hash: &Sha256Hash,
                      _nostr_hash: &bitcoin::hashes::sha256::Hash,
                      name: &Option<Name>,
                      server_urls: &Vec<url::Url>,
                      is_important: &Option<bool>,
                      context: &Vec<FileReferenceContext>| {
                    *hash == attachment.hash
                        && *name == Some(attachment.name.clone())
                        && server_urls.is_empty()
                        && *is_important == Some(true)
                        && context
                            == &vec![FileReferenceContext::Bill {
                                bill_id: bill_id_test().to_string(),
                                field: "files".to_string(),
                            }]
                },
            )
            .returning(
                |hash: &Sha256Hash,
                 nostr_hash: &bitcoin::hashes::sha256::Hash,
                 name: Option<Name>,
                 _: Vec<url::Url>,
                 is_important: Option<bool>,
                 context: Vec<FileReferenceContext>| {
                    let mut file_ref = FileReference::new(hash.clone(), *nostr_hash, name);
                    file_ref.is_important = is_important.unwrap_or(false);
                    file_ref.context = context;
                    Ok(file_ref)
                },
            )
            .once();

        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(bill_store),
            Arc::new(file_reference_store),
            Arc::new(contact),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            bitcoin::Network::Testnet,
        );

        let result = handler.add_new_chain(vec![issue_block], &keys).await;

        assert!(result.is_ok());
    }

    fn create_mocks() -> (
        MockBillChainStore,
        MockBillStore,
        MockNostrContactProcessorApi,
        MockNotificationJsonTransport,
        MockNostrChainEventStore,
    ) {
        (
            MockBillChainStore::new(),
            MockBillStore::new(),
            MockNostrContactProcessorApi::new(),
            MockNotificationJsonTransport::new(),
            MockNostrChainEventStore::new(),
        )
    }

    fn test_privkeys(network: bitcoin::Network) -> (PrivateKey, PrivateKey) {
        let sk1 = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let sk2 = SecretKey::from_slice(&[2u8; 32]).unwrap();

        (PrivateKey::new(sk1, network), PrivateKey::new(sk2, network))
    }

    fn test_pubkeys(network: bitcoin::Network) -> (PublicKey, PublicKey) {
        let (pk1, pk2) = test_privkeys(network);
        (
            pk1.public_key(SECP256K1).inner,
            pk2.public_key(SECP256K1).inner,
        )
    }

    #[tokio::test]
    async fn test_validate_payment_address_accepts_matching_address() {
        let (bill_chain_store, _, _, _, _) = create_mocks();
        let network = bitcoin::Network::Testnet;
        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            network,
        );

        let block_id = BlockId::first().add(6);
        let previous_hash =
            Sha256Hash::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (bill_pub, holder_pub) = test_pubkeys(network);

        let tweak_hash = btc::calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id,
            &previous_hash,
        )
        .unwrap();

        let address =
            btc::get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let result = handler
            .validate_payment_address_for_payment_request(
                &bill_id_test(),
                BillOpCode::RequestToPay,
                &block_id,
                &previous_hash,
                &bill_pub,
                &holder_pub,
                network,
                &address,
                &None,
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_payment_address_mint_case() {
        init_test_cfg();
        let (bill_chain_store, _, _, _, _) = create_mocks();
        let network = bitcoin::Network::Testnet;
        let mut mock_mint_client = MockMintClient::new();
        mock_mint_client
            .expect_validate_payment_address_from_mint()
            .returning(|_, _, _, _, _| Ok(()))
            .times(1);
        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(mock_mint_client),
            bitcoin::Network::Testnet,
        );

        let block_id = BlockId::first().add(6);
        let previous_hash =
            Sha256Hash::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (bill_pub, holder_pub) = test_pubkeys(network);

        let address = valid_payment_address_testnet();

        let result = handler
            .validate_payment_address_for_payment_request(
                &bill_id_test(),
                BillOpCode::RequestToPay,
                &block_id,
                &previous_hash,
                &bill_pub,
                &holder_pub,
                network,
                &address,
                &Some(BillParticipant::Ident(
                    BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
                )),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_payment_address_rejects_non_matching_address() {
        let (bill_chain_store, _, _, _, _) = create_mocks();
        let network = bitcoin::Network::Testnet;
        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            network,
        );

        let block_id = BlockId::first().add(6);
        let previous_hash =
            Sha256Hash::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (bill_pub, holder_pub) = test_pubkeys(network);

        let wrong_address = BitcoinAddress::from_str(
            "tb1p98hgytlecct3qzfmd9qnf05q03ql032xvpdg9kpwfftej2t95t8s0eyx5k",
        )
        .unwrap();

        let err = handler
            .validate_payment_address_for_payment_request(
                &bill_id_test(),
                BillOpCode::RequestToPay,
                &block_id,
                &previous_hash,
                &bill_pub,
                &holder_pub,
                network,
                &wrong_address,
                &None,
            )
            .await
            .expect_err("wrong address should fail");

        match err {
            Error::Crypto(msg) => assert!(msg.contains("don't match")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_validate_payment_address_rejects_when_op_changes() {
        let (bill_chain_store, _, _, _, _) = create_mocks();
        let network = bitcoin::Network::Testnet;
        let handler = BillChainEventProcessor::new(
            Arc::new(bill_chain_store),
            Arc::new(MockBillStore::new()),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockMintClient::new()),
            network,
        );

        let block_id = BlockId::first().add(6);
        let previous_hash =
            Sha256Hash::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let (bill_pub, holder_pub) = test_pubkeys(network);

        let tweak_hash = btc::calculate_tweak_hash_for_payment_request(
            BillOpCode::RequestToPay,
            &block_id,
            &previous_hash,
        )
        .unwrap();

        let address =
            btc::get_address_to_pay(&bill_pub, &holder_pub, &tweak_hash, network).unwrap();

        let err = handler
            .validate_payment_address_for_payment_request(
                &bill_id_test(),
                BillOpCode::OfferToSell,
                &block_id,
                &previous_hash,
                &bill_pub,
                &holder_pub,
                network,
                &address,
                &None,
            )
            .await
            .expect_err("different op should fail");

        match err {
            Error::Crypto(msg) => assert!(msg.contains("don't match")),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
