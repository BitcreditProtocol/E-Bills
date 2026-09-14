use crate::{
    Error, PushApi, Result,
    handler::{
        NotificationHandlerApi,
        public_chain_helpers::{BlockData, is_fork_block, resolve_event_chains, resolve_fork},
    },
};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_api::{get_config, service::transport_service::transport_client::TransportClientApi};
use bcr_ebill_core::{
    application::{
        company::CompanyStatus,
        contact::Contact,
        identity::ActiveIdentityState,
        notification::{Notification, NotificationLevel, NotificationType},
    },
    protocol::{
        EditOptionalFieldMode, Validate,
        blockchain::{Block, bill::ContactType, company::CompanyValidateActionData},
        crypto::BcrKeys,
        event::{ChainInvite, Event},
    },
};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;

use bcr_ebill_core::{
    application::ServiceTraitBounds,
    application::company::{Company, CompanySignatoryStatus},
    protocol::BlockId,
    protocol::blockchain::{
        Blockchain, BlockchainType,
        bill::BillOpCode,
        company::{CompanyBlock, CompanyBlockPayload, CompanyBlockchain},
    },
};
use bcr_ebill_persistence::{
    ContactStoreApi, FileReferenceStoreApi, NostrChainEventStoreApi, NotificationStoreApi,
    company::{CompanyChainStoreApi, CompanyStoreApi},
    identity::IdentityStoreApi,
};

use super::inbound_file_anchor::{anchor_important_file, company_file_context};
use super::{CompanyChainEventProcessorApi, NostrContactProcessorApi};

#[derive(Clone)]
pub struct CompanyChainEventProcessor {
    blockchain_store: Arc<dyn CompanyChainStoreApi>,
    company_store: Arc<dyn CompanyStoreApi>,
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
    identity_store: Arc<dyn IdentityStoreApi>,
    notification_store: Arc<dyn NotificationStoreApi>,
    nostr_contact_processor: Arc<dyn NostrContactProcessorApi>,
    bill_invite_handler: Arc<dyn NotificationHandlerApi>,
    push_service: Arc<dyn PushApi>,
    chain_event_store: Arc<dyn NostrChainEventStoreApi>,
    transport: Arc<dyn TransportClientApi>,
    contact_store: Arc<dyn ContactStoreApi>,
    bitcoin_network: bitcoin::Network,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompanyChainEventProcessorApi for CompanyChainEventProcessor {
    async fn process_chain_data(
        &self,
        company_id: &NodeId,
        blocks: Vec<CompanyBlock>,
        keys: Option<BcrKeys>,
    ) -> Result<()> {
        // check that incoming company blocks are of the same network that we use
        if company_id.network() != self.bitcoin_network {
            warn!("Received company blocks for company {company_id} for a different network");
            return Err(Error::Blockchain(format!(
                "Received company blocks for company {company_id} for a different network"
            )));
        }

        let identity = self
            .identity_store
            .get()
            .await
            .map_err(|e| Error::Persistence(e.to_string()))?
            .node_id;

        if let Ok(mut existing_chain) = self.blockchain_store.get_chain(company_id).await {
            self.add_company_blocks(company_id, &mut existing_chain, blocks, &identity, false)
                .await
        } else {
            match keys {
                Some(keys) => self.add_new_chain(blocks, &keys, &identity).await,
                _ => {
                    error!("Received company blocks for unknown company {company_id}");
                    Err(Error::Blockchain(
                        "Received company blocks for unknown company".to_string(),
                    ))
                }
            }
        }
    }

    async fn validate_chain_event_and_sender(
        &self,
        company_id: &NodeId,
        sender: nostr::key::PublicKey,
    ) -> Result<bool> {
        if let Ok(company) = self
            .company_store
            .get(company_id)
            .await
            .map_err(|e| Error::Persistence(e.to_string()))
        {
            let signers = company
                .signatories
                .iter()
                .map(|s| s.node_id.npub())
                .collect::<Vec<nostr::key::PublicKey>>();
            Ok(signers.contains(&sender))
        } else {
            Ok(false)
        }
    }

    async fn resync_chain(&self, company_id: &NodeId) -> Result<()> {
        match (
            self.blockchain_store.get_chain(company_id).await,
            self.company_store.get_key_pair(company_id).await,
        ) {
            (Ok(mut existing_chain), Ok(company_keys)) => {
                debug!("starting company chain resync for company {company_id}");

                // Pre-fetch identity needed for block processing
                let identity = self
                    .identity_store
                    .get()
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?
                    .node_id;

                if let Ok(chain_data) = resolve_event_chains(
                    self.transport.clone(),
                    &company_id.to_string(),
                    BlockchainType::Company,
                    &Some(company_keys.to_owned()),
                )
                .await
                {
                    for data in chain_data.iter() {
                        let blocks: Vec<CompanyBlock> = data
                            .iter()
                            .filter_map(|d| match d.block.clone() {
                                BlockData::Company(block) => Some(block),
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

                        match self.validate_company_blocks_for_chain(
                            &mut test_chain,
                            &blocks,
                            &company_keys,
                        ) {
                            Ok(()) => {
                                if let Some(fork_id) = fork_point {
                                    info!(
                                        "Fork resolution for company {company_id}: replacing blocks from height {fork_id} with preferred remote chain"
                                    );
                                    if let Err(e) = self
                                        .blockchain_store
                                        .remove_blocks_from_height(company_id, fork_id)
                                        .await
                                    {
                                        error!(
                                            "Failed to remove blocks from height for company {company_id}: {e}"
                                        );
                                        return Err(Error::Persistence(
                                            "Failed to remove blocks from height for fork resolution"
                                                .to_string(),
                                        ));
                                    }
                                    existing_chain.truncate_from(fork_id);
                                }

                                if let Err(e) = self
                                    .add_company_blocks(
                                        company_id,
                                        &mut existing_chain,
                                        blocks,
                                        &identity,
                                        true,
                                    )
                                    .await
                                {
                                    error!(
                                        "Failed to add blocks after truncation for company {company_id}: {e}"
                                    );
                                    return Err(e);
                                }

                                if let Err(e) = self
                                    .chain_event_store
                                    .remove_chain_events(
                                        &company_id.to_string(),
                                        BlockchainType::Company,
                                    )
                                    .await
                                {
                                    error!(
                                        "Failed to invalidate old company chain events during resync: {e}"
                                    );
                                }

                                for event_container in data.iter() {
                                    let event = event_container.as_chain_store_event(
                                        &company_id.to_string(),
                                        BlockchainType::Company,
                                        event_container.block.get_block_height() as usize,
                                    );
                                    if let Err(e) =
                                        self.chain_event_store.add_chain_event(event).await
                                    {
                                        error!(
                                            "Failed to store company chain event during resync: {e}"
                                        );
                                    }
                                }

                                debug!(
                                    "resynced company {company_id} with {} remote events",
                                    data.len()
                                );
                                return Ok(());
                            }
                            Err(e) => {
                                debug!(
                                    "Chain candidate failed validation for company {company_id}: {e}"
                                );
                                continue;
                            }
                        }
                    }

                    debug!("finished company chain resync for {company_id}");
                    Ok(())
                } else {
                    let message =
                        format!("Could not refetch chain data from Nostr for company {company_id}");
                    error!("{message}");
                    Err(Error::Network(message))
                }
            }
            _ => {
                let message = format!(
                    "Could not refetch chain for {company_id} because the company keys or chain could not be fetched"
                );
                error!("{message}");
                Err(Error::Persistence(message))
            }
        }
    }
}

impl CompanyChainEventProcessor {
    pub fn new(
        blockchain_store: Arc<dyn CompanyChainStoreApi>,
        company_store: Arc<dyn CompanyStoreApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        identity_store: Arc<dyn IdentityStoreApi>,
        notification_store: Arc<dyn NotificationStoreApi>,
        nostr_contact_processor: Arc<dyn NostrContactProcessorApi>,
        bill_invite_handler: Arc<dyn NotificationHandlerApi>,
        push_service: Arc<dyn PushApi>,
        chain_event_store: Arc<dyn NostrChainEventStoreApi>,
        transport: Arc<dyn TransportClientApi>,
        contact_store: Arc<dyn ContactStoreApi>,
        bitcoin_network: bitcoin::Network,
    ) -> Self {
        Self {
            blockchain_store,
            company_store,
            file_reference_store,
            identity_store,
            notification_store,
            nostr_contact_processor,
            bill_invite_handler,
            push_service,
            chain_event_store,
            bitcoin_network,
            contact_store,
            transport,
        }
    }

    async fn add_new_chain(
        &self,
        blocks: Vec<CompanyBlock>,
        keys: &BcrKeys,
        identity: &NodeId,
    ) -> Result<()> {
        let (company_id, mut company, mut chain, we_are_signatory_or_invited) =
            self.get_valid_chain(blocks.clone(), keys, identity)?;
        if we_are_signatory_or_invited {
            debug!(
                "adding new chain and company {company_id} with {} blocks",
                blocks.len()
            );
            // add the company
            self.company_store.insert(&company).await.map_err(|e| {
                error!("Failed to insert company {company_id}: {e}");
                Error::Persistence(e.to_string())
            })?;

            // save keys
            self.save_keys(&company_id, keys).await?;

            self.transport
                .add_identity(company_id.clone(), keys.clone())
                .await?;

            // save the first block
            self.save_block(&company_id, chain.get_first_block())
                .await?;

            if let Some(file) = &company.logo_file {
                anchor_important_file(
                    &self.file_reference_store,
                    file,
                    company_file_context(&company.id, "logo_file"),
                )
                .await?;
            }

            if let Some(file) = &company.proof_of_registration_file {
                anchor_important_file(
                    &self.file_reference_store,
                    file,
                    company_file_context(&company.id, "proof_of_registration_file"),
                )
                .await?;
            }

            // save all blocks
            for block in blocks.iter().skip(1) {
                self.add_company_block(
                    &company_id,
                    keys,
                    &mut company,
                    &mut chain,
                    block,
                    identity,
                )
                .await?;
            }

            // also add the company to our nostr contacts
            self.nostr_contact_processor
                .ensure_nostr_contact(&company_id)
                .await;

            // as well as all the company signatories
            for signatory in company.signatories.iter() {
                self.nostr_contact_processor
                    .ensure_nostr_contact(&signatory.node_id)
                    .await;
            }

            // add to contacts without files if it's not there already, but don't fail
            if let Ok(None) = self.contact_store.get(&company_id).await
                && let Err(e) = self
                    .contact_store
                    .insert(
                        &company_id,
                        Contact {
                            t: ContactType::Company,
                            node_id: company_id.to_owned(),
                            name: company.name.clone(),
                            email: Some(company.email.clone()),
                            postal_address: Some(company.postal_address.clone()),
                            date_of_birth_or_registration: company.registration_date.clone(),
                            country_of_birth_or_registration: company
                                .country_of_registration
                                .clone(),
                            city_of_birth_or_registration: company.city_of_registration.clone(),
                            identification_number: company.registration_number.clone(),
                            avatar_file: None,
                            proof_document_file: None,
                            nostr_relays: get_config().nostr_config.relays.clone(),
                            is_logical: false,
                            mint_url: Some(get_config().mint_config.default_mint_url.clone()),
                        },
                    )
                    .await
            {
                error!(
                    "Could not create contact for company {company_id} identity {identity} was added for: {e}"
                );
            }
        } else {
            info!("We are not a signatory for company {company_id} so skipping chain");
        }
        Ok(())
    }

    async fn add_company_blocks(
        &self,
        company_id: &NodeId,
        chain: &mut CompanyBlockchain,
        blocks: Vec<CompanyBlock>,
        identity: &NodeId,
        from_resync: bool,
    ) -> Result<()> {
        let keys = self
            .company_store
            .get_key_pair(company_id)
            .await
            .map_err(|e| Error::Persistence(e.to_string()))?;
        let mut company = self
            .company_store
            .get(company_id)
            .await
            .map_err(|e| Error::Persistence(e.to_string()))?;

        let mut block_height = chain.get_latest_block().id;
        for block in blocks.iter() {
            if block.id <= block_height {
                if blocks.len() == 1 && !from_resync {
                    let latest = chain.get_latest_block();
                    if is_fork_block(latest, block) {
                        info!(
                            "Split chain detected for company {company_id} at height {} - resyncing",
                            block.id
                        );
                        self.resync_chain(company_id).await?;
                        return Ok(());
                    }
                }
                info!(
                    "Skipping block with id {block_height} for {company_id} as we already have it"
                );
                continue;
            }

            match self
                .add_company_block(company_id, &keys, &mut company, chain, block, identity)
                .await
            {
                Ok(_) => {
                    block_height = block.id;
                    Ok(())
                }
                Err(e) => {
                    // if we received a single block (normal block populate) and we are missing blocks, we try to resync
                    if blocks.len() == 1
                        && !from_resync
                        && BlockId::next_from_previous_block_id(&chain.get_latest_block().id)
                            < block.id
                    {
                        info!(
                            "Received invalid block {} for company {company_id} - missing blocks - try to resync",
                            block.id
                        );
                        self.resync_chain(company_id).await?;
                        break;
                    } else {
                        error!("Error adding block for company {company_id}: {e}");
                        Err(e)
                    }
                }
            }?;
        }
        debug!("Updated company {company_id} with data from new blocks");
        Ok(())
    }

    /// Validates a single company block for a chain WITHOUT persisting or processing side effects.
    /// Returns true if the block was validated and added to the in-memory chain.
    /// Returns false if the block should be skipped (already exists, etc.).
    fn validate_company_block_for_chain(
        &self,
        chain: &mut CompanyBlockchain,
        block: &CompanyBlock,
        company_keys: &BcrKeys,
    ) -> Result<bool> {
        let block_height = chain.get_latest_block().id;

        // if we already have the block, we skip it
        if block.id <= block_height {
            return Ok(false);
        }
        let chain_clone_for_validation = chain.clone();

        // do cheap integrity checks (mutates chain in-memory)
        if !chain.try_add_block(block.clone()) {
            error!("Received invalid company block");
            return Err(Error::Blockchain(
                "Received invalid company block".to_string(),
            ));
        }
        let company_id = block.company_id.clone();
        let block_id = block.id;

        // then, verify signature and signer of the block and get data to validate
        let verify_and_get_signer = match block.verify_and_get_signer(company_keys) {
            Ok(d) => d,
            Err(e) => {
                error!(
                    "Received invalid block {block_id} for company {company_id} - could not verify signature from block data signer"
                );
                return Err(Error::Blockchain(e.to_string()));
            }
        };

        if let Err(e) = (CompanyValidateActionData {
            blockchain: chain_clone_for_validation,
            company_id: company_id.clone(),
            signer_node_id: verify_and_get_signer.signer,
            op: block.op_code().clone(),
            company_keys: company_keys.to_owned(),
            invitee: verify_and_get_signer.invitee,
            removee: verify_and_get_signer.removee,
            identity_proof_data: verify_and_get_signer.identity_proof_data,
        })
        .validate()
        {
            error!(
                "Received invalid block {block_id} for company {company_id}, company action validation failed: {e}"
            );
            return Err(Error::Blockchain(e.to_string()));
        }

        Ok(true)
    }

    /// Validates multiple company blocks for a chain WITHOUT persisting or processing side effects.
    /// Returns Ok(()) if all blocks are valid, Err otherwise.
    fn validate_company_blocks_for_chain(
        &self,
        chain: &mut CompanyBlockchain,
        blocks: &[CompanyBlock],
        company_keys: &BcrKeys,
    ) -> Result<()> {
        for block in blocks {
            self.validate_company_block_for_chain(chain, block, company_keys)?;
        }
        Ok(())
    }

    async fn add_company_block(
        &self,
        company_id: &NodeId,
        keys: &BcrKeys,
        company: &mut Company,
        chain: &mut CompanyBlockchain,
        block: &CompanyBlock,
        identity_node_id: &NodeId,
    ) -> Result<()> {
        if self.validate_company_block_for_chain(chain, block, keys)? {
            let data = block
                .get_block_data(keys)
                .map_err(|e| Error::Blockchain(e.to_string()))?;

            // process effects
            self.process_company_block_effects(
                company_id,
                company,
                data,
                identity_node_id,
                block.timestamp(),
            )
            .await?;

            // persist data
            self.save_block(company_id, block).await?;
        }
        Ok(())
    }

    /// Processes side effects for a company block (contacts, updates, invites, etc.) WITHOUT persisting the block itself.
    async fn process_company_block_effects(
        &self,
        company_id: &NodeId,
        company: &mut Company,
        data: CompanyBlockPayload,
        identity_node_id: &NodeId,
        timestamp: bcr_ebill_core::protocol::Timestamp,
    ) -> Result<()> {
        match data {
            CompanyBlockPayload::Create(_) => { /* creates are handled on validation */ }
            update @ CompanyBlockPayload::Update(_) => {
                let files_to_anchor = match &update {
                    CompanyBlockPayload::Update(payload) => vec![
                        match &payload.logo_file {
                            EditOptionalFieldMode::Set(file) => {
                                Some((file.clone(), company_file_context(company_id, "logo_file")))
                            }
                            _ => None,
                        },
                        match &payload.proof_of_registration_file {
                            EditOptionalFieldMode::Set(file) => Some((
                                file.clone(),
                                company_file_context(company_id, "proof_of_registration_file"),
                            )),
                            _ => None,
                        },
                    ],
                    _ => vec![],
                };
                info!("Updating company {company_id} from block data");
                company.apply_block_data(&update, identity_node_id, timestamp);
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;

                for (file, context) in files_to_anchor.into_iter().flatten() {
                    anchor_important_file(&self.file_reference_store, &file, context).await?;
                }
            }
            CompanyBlockPayload::InviteSignatory(payload) => {
                info!("Signatory invited to company {company_id}, adding to contacts");
                self.nostr_contact_processor
                    .ensure_nostr_contact(&payload.invitee)
                    .await;
                company.apply_block_data(
                    &CompanyBlockPayload::InviteSignatory(payload.clone()),
                    identity_node_id,
                    timestamp,
                );
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;

                // if we're invited, create a notification
                if &payload.invitee == identity_node_id
                    && let Err(e) = self
                        .create_notification(
                            company_id,
                            identity_node_id,
                            &CompanyBlockPayload::InviteSignatory(payload.clone()),
                        )
                        .await
                {
                    error!("Couldn't create notification for company invite for {company_id}: {e}");
                }

                for signatory in company.signatories.iter() {
                    if signatory.node_id != payload.inviter
                        && signatory.node_id != payload.invitee
                        && matches!(
                            signatory.status,
                            CompanySignatoryStatus::InviteAcceptedIdentityProven { .. }
                        )
                        && let Err(e) = self
                            .save_company_notification(
                                company_id,
                                &signatory.node_id,
                                &format!(
                                    "{} invited {} to become a signatory",
                                    payload.inviter, payload.invitee
                                ),
                                Some(serde_json::to_value(&company)?),
                                NotificationLevel::Informational,
                            )
                            .await
                    {
                        error!(
                            "Couldn't create notification for signatory {} about company invite for {company_id}: {e}",
                            signatory.node_id
                        );
                    }
                }

                // reset local hiding state for the invited user, so it's shown again
                if let Err(e) = self
                    .company_store
                    .delete_local_signatory_override(company_id, &payload.invitee)
                    .await
                {
                    warn!(
                        "Couldn't reset local signatory override for company {company_id} and {}: {e}",
                        payload.invitee
                    );
                }
            }
            CompanyBlockPayload::SignatoryAcceptInvite(payload) => {
                info!("Signatory accepted invite to company {company_id}");
                self.nostr_contact_processor
                    .ensure_nostr_contact(&payload.accepter)
                    .await;
                company.apply_block_data(
                    &CompanyBlockPayload::SignatoryAcceptInvite(payload.clone()),
                    identity_node_id,
                    timestamp,
                );
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;

                if let Some(inviter) = company
                    .signatories
                    .iter()
                    .find(|s| matches!(&s.status, CompanySignatoryStatus::Invited { inviter, .. } if *inviter == payload.accepter))
                    .map(|s| s.node_id.clone())
                {
                    for signatory in company.signatories.iter() {
                        if signatory.node_id != payload.accepter
                            && matches!(
                                signatory.status,
                                CompanySignatoryStatus::InviteAcceptedIdentityProven { .. }
                            )
                        {
                            let desc = if signatory.node_id == inviter {
                                format!("{} accepted your invitation to become a signatory", payload.accepter)
                            } else {
                                format!("{} accepted the invitation to become a signatory", payload.accepter)
                            };
                            if let Err(e) = self
                                .save_company_notification(
                                    company_id,
                                    &signatory.node_id,
                                    &desc,
                                    Some(serde_json::to_value(&company)?),
                                    NotificationLevel::Informational,
                                )
                                .await
                            {
                                error!("Couldn't create notification for signatory {} about invite accept for {company_id}: {e}", signatory.node_id);
                            }
                        }
                    }
                }
            }
            CompanyBlockPayload::SignatoryRejectInvite(payload) => {
                info!("Signatory rejected invite to company {company_id}");
                company.apply_block_data(
                    &CompanyBlockPayload::SignatoryRejectInvite(payload.clone()),
                    identity_node_id,
                    timestamp,
                );
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;

                if let Some(inviter) = company
                    .signatories
                    .iter()
                    .find(|s| matches!(&s.status, CompanySignatoryStatus::Invited { inviter, .. } if *inviter == payload.rejecter))
                    .map(|s| s.node_id.clone())
                {
                    for signatory in company.signatories.iter() {
                        if signatory.node_id != payload.rejecter
                            && matches!(
                                signatory.status,
                                CompanySignatoryStatus::InviteAcceptedIdentityProven { .. }
                            )
                        {
                            let desc = if signatory.node_id == inviter {
                                format!("{} rejected your invitation to become a signatory", payload.rejecter)
                            } else {
                                format!("{} rejected the invitation to become a signatory", payload.rejecter)
                            };
                            if let Err(e) = self
                                .save_company_notification(
                                    company_id,
                                    &signatory.node_id,
                                    &desc,
                                    Some(serde_json::to_value(&company)?),
                                    NotificationLevel::Informational,
                                )
                                .await
                            {
                                error!("Couldn't create notification for signatory {} about invite reject for {company_id}: {e}", signatory.node_id);
                            }
                        }
                    }
                }
            }
            CompanyBlockPayload::RemoveSignatory(payload) => {
                let removee = payload.removee.clone();
                info!("Removing signatory from company {company_id}");
                company.apply_block_data(
                    &CompanyBlockPayload::RemoveSignatory(payload.clone()),
                    identity_node_id,
                    timestamp,
                );
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;

                // if we're being removed and current identity is that company - change it
                if &removee == identity_node_id
                    && let Ok(Some(active_node_id)) = self
                        .identity_store
                        .get_current_identity()
                        .await
                        .map(|i| i.company)
                    && &active_node_id == company_id
                    && let Err(e) = self
                        .identity_store
                        .set_current_identity(&ActiveIdentityState {
                            personal: identity_node_id.to_owned(),
                            company: None,
                        })
                        .await
                {
                    error!(
                        "Couldn't set active identity to personal after removing self from company: {e}"
                    );
                }

                if let Err(e) = self
                    .create_notification(
                        company_id,
                        &payload.removee,
                        &CompanyBlockPayload::RemoveSignatory(payload.clone()),
                    )
                    .await
                {
                    error!(
                        "Couldn't create notification for removed signatory {} from company {company_id}: {e}",
                        payload.removee
                    );
                }

                for signatory in company.signatories.iter() {
                    if signatory.node_id != payload.remover
                        && signatory.node_id != payload.removee
                        && matches!(
                            signatory.status,
                            CompanySignatoryStatus::InviteAcceptedIdentityProven { .. }
                        )
                        && let Err(e) = self
                            .create_notification(
                                company_id,
                                &signatory.node_id,
                                &CompanyBlockPayload::RemoveSignatory(payload.clone()),
                            )
                            .await
                    {
                        error!(
                            "Couldn't create notification for signatory {} about removal from company {company_id}: {e}",
                            signatory.node_id
                        );
                    }
                }
            }
            CompanyBlockPayload::SignBill(payload) => {
                if let Some(bill_key) = payload.bill_key
                    && payload.operation == BillOpCode::Issue
                {
                    info!("Adding detected company bill {}", payload.bill_id);
                    let bill_keys = BcrKeys::from_private_key(&bill_key);
                    let invite = ChainInvite::bill(
                        payload.bill_id.to_string(),
                        BcrKeys::from_private_key(&bill_keys.get_private_key()),
                    );
                    // we want to process all blocks for the company even if we don't have all
                    // the bills.
                    if let Err(e) = self
                        .bill_invite_handler
                        .handle_event(Event::new_bill(invite).try_into()?, &company.id, None, None)
                        .await
                    {
                        error!(
                            "Failed to add company bill {} when adding company: {e}",
                            payload.bill_id
                        );
                    }
                }
            }
            CompanyBlockPayload::IdentityProof(payload) => {
                info!("Received identity proof for company {company_id}");
                // if it's our own block - update our local confirmation state
                if &payload.data.node_id == identity_node_id {
                    self.company_store
                        .set_email_confirmation(company_id, &payload.proof, &payload.data)
                        .await
                        .map_err(|e| Error::Persistence(e.to_string()))?;
                }

                // update company data
                company.apply_block_data(
                    &CompanyBlockPayload::IdentityProof(payload),
                    identity_node_id,
                    timestamp,
                );
                self.company_store
                    .update(company_id, company)
                    .await
                    .map_err(|e| Error::Persistence(e.to_string()))?;
            }
        }
        Ok(())
    }
    fn get_valid_chain(
        &self,
        blocks: Vec<CompanyBlock>,
        keys: &BcrKeys,
        identity_node_id: &NodeId,
    ) -> Result<(NodeId, Company, CompanyBlockchain, bool)> {
        match CompanyBlockchain::new_from_blocks(blocks.clone()) {
            Ok(chain) if chain.is_chain_valid() => {
                let first_block = chain.get_first_block();
                // create block is where we build up the company from
                let payload = match first_block
                    .get_block_data(keys)
                    .map_err(|e| Error::Blockchain(e.to_string()))?
                {
                    CompanyBlockPayload::Create(payload) => payload,
                    _ => {
                        error!("First block of newly received company chain is not a Create block");
                        return Err(Error::Blockchain(
                            "First block of newly received company chain is not a Create block"
                                .to_string(),
                        ));
                    }
                };

                // extracted node_id
                let node_id = payload.id.clone();

                // initialize company from payload
                let company = Company::from_block_data(payload, identity_node_id);

                // check if we are a signatory, or to be invited as a signatory
                let we_are_signatory_or_invited = {
                    let mut aggregate = company.clone();
                    for block in blocks.iter().skip(1) {
                        if let Ok(data) = &block.get_block_data(keys) {
                            aggregate.apply_block_data(data, identity_node_id, block.timestamp());
                        }
                    }
                    aggregate.status == CompanyStatus::Active
                        || aggregate.status == CompanyStatus::Invited
                };

                // chain with just the create block
                let return_chain = CompanyBlockchain::new_from_blocks(vec![first_block.clone()])
                    .map_err(|e| Error::Blockchain(e.to_string()))?;

                Ok((node_id, company, return_chain, we_are_signatory_or_invited))
            }
            _ => {
                error!("Newly received company chain is not valid");
                Err(Error::Blockchain(
                    "Newly received company chain is not valid".to_string(),
                ))
            }
        }
    }

    async fn save_block(&self, company_id: &NodeId, block: &CompanyBlock) -> Result<()> {
        if let Err(e) = self.blockchain_store.add_block(company_id, block).await {
            error!("Failed to add company block to blockchain store: {e}");
            return Err(Error::Persistence(
                "Failed to add company block to blockchain store".to_string(),
            ));
        }
        Ok(())
    }

    async fn save_keys(&self, node_id: &NodeId, keys: &BcrKeys) -> Result<()> {
        if let Err(e) = self.company_store.save_key_pair(node_id, keys).await {
            error!("Failed to save keys to company store: {e}");
            return Err(Error::Persistence(
                "Failed to save keys to company store".to_string(),
            ));
        }
        Ok(())
    }

    async fn create_notification(
        &self,
        company_id: &NodeId,
        node_id: &NodeId,
        payload: &CompanyBlockPayload,
    ) -> Result<()> {
        let company = self
            .company_store
            .get(company_id)
            .await
            .map_err(|e| Error::Persistence(e.to_string()))?;
        let (description, event, level) = match payload {
            CompanyBlockPayload::InviteSignatory(_) => {
                let desc = format!(
                    "{} have requested you to become an authorised signer",
                    company.name
                );
                (
                    desc,
                    Some(serde_json::to_value(&company)?),
                    NotificationLevel::ActionRequired,
                )
            }
            CompanyBlockPayload::RemoveSignatory(payload) => {
                let desc = if node_id == &payload.removee {
                    format!("You were removed from {}", company.name)
                } else {
                    format!("{} was removed from {}", payload.removee, company.name)
                };
                (
                    desc,
                    Some(serde_json::to_value(&company)?),
                    NotificationLevel::Informational,
                )
            }
            _ => return Ok(()), // no notifications for these yet
        };

        self.save_company_notification(company_id, node_id, &description, event, level)
            .await
    }

    async fn save_company_notification(
        &self,
        company_id: &NodeId,
        node_id: &NodeId,
        description: &str,
        event: Option<serde_json::Value>,
        level: NotificationLevel,
    ) -> Result<()> {
        debug!(
            "Company notification: company_id={} recipient={} level={:?} desc=\"{}\"",
            company_id, node_id, level, description
        );
        let notification =
            Notification::new_company_notification(company_id, node_id, description, event, level);

        // mark Company event as done if any active one exists
        // BUT only if we're not demoting an ActionRequired notification to Informational
        match self
            .notification_store
            .get_latest_by_reference_and_node_id(
                &company_id.to_string(),
                NotificationType::Company,
                node_id,
            )
            .await
        {
            Ok(Some(currently_active)) => {
                let should_replace = match (currently_active.level, level) {
                    // Never demote ActionRequired to Informational
                    (NotificationLevel::ActionRequired, NotificationLevel::Informational) => {
                        trace!(
                            "Skipping company notification for {company_id}: would demote ActionRequired to Informational"
                        );
                        false
                    }
                    _ => true,
                };
                if should_replace
                    && let Err(e) = self
                        .notification_store
                        .mark_as_done(&currently_active.id)
                        .await
                {
                    error!("Failed to mark currently active notification as done: {e}");
                }
            }
            Err(e) => error!("Failed to get latest notification by reference: {e}"),
            Ok(None) => {}
        }

        // save new notification to database
        self.notification_store
            .add(notification.clone())
            .await
            .map_err(|e| {
                error!("Failed to save new notification to database: {e}");
                Error::Persistence("Failed to save new notification to database".to_string())
            })?;

        // send push notification to connected clients
        match serde_json::to_value(notification) {
            Ok(notification) => {
                trace!("sending notification {notification:?} for {node_id}");
                self.push_service.send(notification).await;
            }
            Err(e) => {
                error!("Failed to serialize notification for push service: {e}");
            }
        }
        Ok(())
    }
}

impl ServiceTraitBounds for CompanyChainEventProcessor {}

#[cfg(test)]
#[allow(unused_mut)]
pub mod tests {
    use std::sync::Arc;

    use bcr_common::core::NodeId;
    use bcr_ebill_core::application::company::CompanySignatoryStatus;
    use bcr_ebill_core::protocol::blockchain::Block;
    use bcr_ebill_core::protocol::blockchain::company::CompanyBlockPayload;
    use bcr_ebill_core::protocol::blockchain::company::block::{
        CompanyCreateBlockData, CompanySignatoryAcceptInviteBlockData,
        CompanySignatoryRejectInviteBlockData,
    };
    use bcr_ebill_core::protocol::event::{CompanyBlockEvent, Event, EventEnvelope};
    use bcr_ebill_core::protocol::file_reference::{FileReference, FileReferenceContext};
    use bcr_ebill_core::protocol::{EditOptionalFieldMode, File, Name};
    use bcr_ebill_core::{
        application::company::Company,
        application::identity::ActiveIdentityState,
        application::notification::{Notification, NotificationLevel},
        protocol::Sha256Hash,
        protocol::Timestamp,
        protocol::blockchain::{
            Blockchain, BlockchainType,
            company::{
                CompanyBlock, CompanyBlockchain,
                block::{
                    CompanyIdentityProofBlockData, CompanyInviteSignatoryBlockData,
                    CompanyRemoveSignatoryBlockData, CompanyUpdateBlockData, SignatoryType,
                },
            },
        },
        protocol::crypto::BcrKeys,
    };
    use mockall::predicate::{always, eq};
    use nostr::event::FinalizeEvent;

    use crate::handler::test_utils::{
        MockNotificationStore, get_valid_activated_signatory, node_id_test_another,
        private_key_test, private_key_test_another, update_company_block_with_name,
    };
    use crate::push_notification::MockPushApi;
    use crate::test_utils::{
        MockContactStore, MockFileReferenceStore, signed_identity_proof_test, test_ts,
    };
    use crate::{
        handler::{
            CompanyChainEventProcessor, CompanyChainEventProcessorApi,
            MockNostrContactProcessorApi, MockNotificationHandlerApi,
            test_utils::{
                MockCompanyChainStore, MockCompanyStore, MockIdentityStore,
                MockNostrChainEventStore, get_company_data, node_id_test,
            },
        },
        test_utils::{MockNotificationJsonTransport, get_baseline_identity},
        transport::create_public_chain_event,
    };

    #[tokio::test]
    async fn test_create_event_handler() {
        let (
            chain_store,
            store,
            notification_store,
            contact,
            bill,
            identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );
    }

    #[tokio::test]
    async fn test_validate_chain_event_and_sender_invalid_on_no_keys_or_chain() {
        let keys = BcrKeys::new().get_nostr_keys();
        let (
            chain_store,
            mut store,
            notification_store,
            contact,
            bill,
            identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();

        store
            .expect_get()
            .with(eq(node_id_test()))
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "company block".to_string(),
                    node_id_test().to_string(),
                ))
            });

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        let valid = handler
            .validate_chain_event_and_sender(&node_id_test(), keys.public_key())
            .await
            .expect("Event should be handled");
        assert!(!valid);
    }

    #[tokio::test]
    async fn test_validate_chain_event_fails_if_not_signatory() {
        let keys = BcrKeys::new();
        let (
            chain_store,
            mut store,
            notification_store,
            contact,
            bill,
            identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let (_, (company, _)) = get_company_data();
        store
            .expect_get()
            .with(eq(node_id_test()))
            .returning(move |_| Ok(company.clone()));

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        let valid = handler
            .validate_chain_event_and_sender(&node_id_test(), keys.get_nostr_keys().public_key())
            .await
            .expect("Event should be handled");
        assert!(!valid);
    }

    #[tokio::test]
    async fn test_validate_chain_event() {
        let keys = BcrKeys::new();
        let (
            chain_store,
            mut store,
            notification_store,
            contact,
            bill,
            identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let (_, (mut company, _)) = get_company_data();
        company.signatories = vec![get_valid_activated_signatory(&NodeId::new(
            keys.pub_key(),
            bitcoin::Network::Testnet,
        ))];
        store
            .expect_get()
            .with(eq(node_id_test()))
            .returning(move |_| Ok(company.clone()));

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        let valid = handler
            .validate_chain_event_and_sender(&node_id_test(), keys.get_nostr_keys().public_key())
            .await
            .expect("Event should be handled");
        assert!(valid);
    }

    #[tokio::test]
    async fn test_process_create_company_data() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            mut transport,
            push_service,
            mut contact_store,
        ) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let _blocks = [get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];

        let node_id_clone = node_id.clone();
        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| {
                Err(bcr_ebill_persistence::Error::NoSuchEntity(
                    "company block".to_string(),
                    node_id_clone.to_string(),
                ))
            })
            .once();

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        contact_store.expect_get().returning(|_| Ok(None));
        contact_store.expect_insert().returning(|_, _| Ok(()));

        // it is valid so add the company
        let expected_company = company.clone();
        store
            .expect_insert()
            .withf(move |c| c.id == expected_company.id.clone() && c.name == expected_company.name)
            .returning(|_| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // adds the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        transport
            .expect_add_identity()
            .with(eq(node_id.clone()), eq(keys.clone()))
            .returning(|_, _| Ok(()))
            .once();

        // and ensures that we have all nostr contacts for the company participants
        contact
            .expect_ensure_nostr_contact()
            .with(eq(node_id.clone()))
            .returning(|_| ())
            .times(2);

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        let block = get_company_create_block(node_id.clone(), company.clone(), &keys);
        handler
            .process_chain_data(&node_id, vec![block], Some(keys.clone()))
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_process_update_company_data() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        chain = add_creator_identity_proof_block(chain);
        let data = update_company_block_with_name(Some(Name::new("new_name").unwrap()));
        let update_block = get_company_update_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .once();

        // apply changes from block and update the company
        let expected_node = node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.name == Name::new("new_name").unwrap()
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // no need to ensure contacts in case of an update
        contact
            .expect_ensure_nostr_contact()
            .with(eq(node_id.clone()))
            .returning(|_| ())
            .never();

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_recovers_chain_on_missing_blocks() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            mut chain_event_store,
            mut transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let bcr_keys: BcrKeys = keys.clone();
        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut skipped_chain =
            CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        skipped_chain = add_creator_identity_proof_block(skipped_chain);
        let data_skipped = update_company_block_with_name(Some(Name::new("new_name").unwrap()));
        let skipped_block = get_company_update_block(
            node_id.clone(),
            skipped_chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data_skipped,
        );

        let mut full_chain = skipped_chain.clone();
        assert!(full_chain.try_add_block(skipped_block.clone()));

        let data = update_company_block_with_name(Some(Name::new("another_name").unwrap()));
        let update_block = get_company_update_block(
            node_id.clone(),
            full_chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );

        let event1 = generate_test_event(
            &bcr_keys,
            None,
            None,
            as_event_payload(&node_id, skipped_chain.get_first_block()),
            &node_id,
        );

        let event2 = generate_test_event(
            &bcr_keys,
            Some(event1.clone()),
            Some(event1.clone()),
            as_event_payload(&node_id, skipped_chain.get_latest_block()),
            &node_id,
        );

        let event3 = generate_test_event(
            &bcr_keys,
            Some(event2.clone()),
            Some(event1.clone()),
            as_event_payload(&node_id, &skipped_block),
            &node_id,
        );

        let event4 = generate_test_event(
            &bcr_keys,
            Some(event3.clone()),
            Some(event1.clone()),
            as_event_payload(&node_id, &update_block),
            &node_id,
        );

        let nostr_chain = vec![
            event1.clone(),
            event2.clone(),
            event3.clone(),
            event4.clone(),
        ];

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .times(2);

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(skipped_chain.clone()))
            .times(2);

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .times(3);

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .times(2);

        // and queries the chain from the transport
        transport
            .expect_resolve_public_chain()
            .with(eq(node_id.to_string()), eq(BlockchainType::Company))
            .returning(move |_, _| Ok(nostr_chain.clone()))
            .once();

        // apply changes from skipped block and update the company
        let expected_node = node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.name == Name::new("new_name").unwrap()
            })
            .returning(|_, _| Ok(()))
            .once();

        // apply changes from last block and update the company
        let expected_node = node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.name == Name::new("another_name").unwrap()
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .times(2);

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // no need to ensure contacts in case of an update
        contact
            .expect_ensure_nostr_contact()
            .with(eq(node_id.clone()))
            .returning(|_| ())
            .never();

        chain_event_store
            .expect_remove_chain_events()
            .returning(|_, _| Ok(()));

        chain_event_store
            .expect_add_chain_event()
            .returning(|_| Ok(()))
            .times(0..);

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    fn as_event_payload(id: &NodeId, block: &CompanyBlock) -> EventEnvelope {
        Event::new_company_chain(CompanyBlockEvent {
            node_id: id.clone(),
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
        node_id: &NodeId,
    ) -> nostr::event::Event {
        create_public_chain_event(
            &node_id.to_string(),
            data,
            Timestamp::new(1000).unwrap(),
            BlockchainType::Company,
            previous,
            root,
        )
        .expect("could not create chain event")
        .finalize(&keys.get_nostr_keys())
        .expect("could not sign event")
    }

    #[tokio::test]
    async fn test_process_invite_company_signatory() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let new_node_id = node_id_test_another();
        let (node_id, (company, keys)) = get_company_data();
        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        chain = add_creator_identity_proof_block(chain);
        let data = CompanyInviteSignatoryBlockData {
            invitee: new_node_id.clone(),
            inviter: node_id_test(),
            t: SignatoryType::Solo,
        };
        let update_block = get_company_invite_signatory_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        // if node is invited, we clean local overrides
        store
            .expect_delete_local_signatory_override()
            .with(eq(node_id.clone()), eq(new_node_id.clone()))
            .returning(|_, _| Ok(()))
            .once();

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .once();

        // apply changes from block with the signatory
        let expected_node = node_id.clone();
        let expected_new_node = new_node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.signatories
                        .iter()
                        .any(|s| s.node_id == expected_new_node.clone())
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // ensure the new node is a contact
        contact
            .expect_ensure_nostr_contact()
            .with(eq(new_node_id.clone()))
            .returning(|_| ())
            .once();

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_process_accept_company_invite() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let new_node_id = node_id_test_another();
        let (node_id, (mut company, keys)) = get_company_data();
        company
            .signatories
            .push(get_valid_activated_signatory(&new_node_id.clone()));

        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        chain = add_invite_signatory_block(chain);
        let data = CompanySignatoryAcceptInviteBlockData {
            accepter: new_node_id.clone(),
        };
        let update_block = get_company_accept_invite_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test_another()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .once();

        // apply changes from block with the signatory
        let expected_node = node_id.clone();
        let expected_new_node = new_node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.signatories.iter().any(|s| {
                        s.node_id == expected_new_node.clone()
                            && matches!(s.status, CompanySignatoryStatus::InviteAccepted { .. })
                    })
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // ensure the new node is a contact
        contact
            .expect_ensure_nostr_contact()
            .with(eq(new_node_id.clone()))
            .returning(|_| ())
            .once();

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_process_reject_company_invite() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let new_node_id = node_id_test_another();
        let (node_id, (mut company, keys)) = get_company_data();
        company
            .signatories
            .push(get_valid_activated_signatory(&new_node_id.clone()));

        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        chain = add_invite_signatory_block(chain);
        let data = CompanySignatoryRejectInviteBlockData {
            rejecter: new_node_id.clone(),
        };
        let update_block = get_company_reject_invite_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test_another()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .once();

        // apply changes from block with the signatory
        let expected_node = node_id.clone();
        let expected_new_node = new_node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.signatories.iter().any(|s| {
                        s.node_id == expected_new_node.clone()
                            && matches!(s.status, CompanySignatoryStatus::InviteRejected { .. })
                    })
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // no need to ensure contacts in case of a reject
        contact
            .expect_ensure_nostr_contact()
            .with(eq(new_node_id.clone()))
            .returning(|_| ())
            .never();

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_process_remove_company_signatory() {
        let (
            mut chain_store,
            mut store,
            mut notification_store,
            mut contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            mut push_service,
            contact_store,
        ) = create_mocks();
        let new_node_id = node_id_test_another();
        let (node_id, (mut company, keys)) = get_company_data();
        company
            .signatories
            .push(get_valid_activated_signatory(&new_node_id.clone()));

        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let mut chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        chain = add_creator_identity_proof_block(chain);
        chain = add_invite_signatory_block(chain);
        let data = CompanyRemoveSignatoryBlockData {
            remover: node_id_test(),
            removee: new_node_id.clone(),
        };
        let remove_block = get_company_remove_signatory_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .times(2);

        // apply changes from block with the signatory
        let expected_node = node_id.clone();
        let expected_new_node = new_node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.signatories.iter().any(|s| {
                        s.node_id == expected_new_node.clone()
                            && matches!(s.status, CompanySignatoryStatus::Removed { .. })
                    })
            })
            .returning(|_, _| Ok(()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        // no need to ensure contacts in case of a remove
        contact
            .expect_ensure_nostr_contact()
            .with(eq(new_node_id.clone()))
            .returning(|_| ())
            .never();

        notification_store
            .expect_get_latest_by_reference_and_node_id()
            .returning(|_, _, _| Ok(None));

        notification_store.expect_add().returning(|_| {
            Ok(Notification::new_company_notification(
                &node_id_test(),
                &node_id_test_another(),
                "test",
                None,
                NotificationLevel::Informational,
            ))
        });

        push_service.expect_send().returning(|_| ());

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![remove_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    #[tokio::test]
    async fn test_process_identity_proof() {
        let (
            mut chain_store,
            mut store,
            notification_store,
            contact,
            bill,
            mut identity,
            chain_event_store,
            transport,
            push_service,
            contact_store,
        ) = create_mocks();
        let (node_id, (company, keys)) = get_company_data();
        let blocks = vec![get_company_create_block(
            node_id.clone(),
            company.clone(),
            &keys,
        )];
        let chain = CompanyBlockchain::new_from_blocks(blocks).expect("could not create chain");
        let (proof, mut d) = signed_identity_proof_test();
        d.company_node_id = Some(node_id.clone());
        let data = CompanyIdentityProofBlockData {
            proof,
            data: d,
            reference_block: None,
        };
        let update_block = get_company_identity_proof_block(
            node_id.clone(),
            chain.get_latest_block(),
            &BcrKeys::from_private_key(&private_key_test()),
            &keys,
            &data,
        );

        // we need to validate if we are a signatory with our identity
        identity
            .expect_get()
            .returning(|| Ok(get_baseline_identity().identity))
            .once();

        // incoming identity proof is set
        store
            .expect_set_email_confirmation()
            .returning(|_, _, _| Ok(()))
            .once();

        // apply changes from block with the identity proof
        let expected_node = node_id.clone();
        let expected_new_node = data.data.node_id.clone();
        store
            .expect_update()
            .withf(move |n, c| {
                n == &expected_node.clone()
                    && c.id == expected_node.clone()
                    && c.signatories.iter().any(|s| {
                        s.node_id == expected_new_node.clone()
                            && matches!(
                                s.status,
                                CompanySignatoryStatus::InviteAcceptedIdentityProven { .. }
                            )
                    })
            })
            .returning(|_, _| Ok(()))
            .once();

        // checks if we already have the chain
        chain_store
            .expect_get_chain()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(chain.clone()))
            .once();

        // we have a chain so get the keys
        store
            .expect_get_key_pair()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(keys.clone()))
            .once();

        // get the current company state
        let expected_company = company.clone();
        store
            .expect_get()
            .with(eq(node_id.clone()))
            .returning(move |_| Ok(expected_company.clone()))
            .once();

        // inserts the block
        chain_store
            .expect_add_block()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .once();

        // we already have the keys
        store
            .expect_save_key_pair()
            .with(eq(node_id.clone()), always())
            .returning(|_, _| Ok(()))
            .never();

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity),
            Arc::new(notification_store),
            Arc::new(contact),
            Arc::new(bill),
            Arc::new(push_service),
            Arc::new(chain_event_store),
            Arc::new(transport),
            Arc::new(contact_store),
            bitcoin::Network::Testnet,
        );

        handler
            .process_chain_data(&node_id, vec![update_block], None)
            .await
            .expect("Process chain data should be handled");
    }

    pub fn get_company_create_block(
        node_id: NodeId,
        company: Company,
        keys: &BcrKeys,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_create(
            node_id,
            Sha256Hash::new("genesis hash"),
            &CompanyCreateBlockData {
                id: company.id,
                name: company.name,
                country_of_registration: company.country_of_registration,
                city_of_registration: company.city_of_registration,
                postal_address: company.postal_address,
                email: company.email,
                registration_number: company.registration_number,
                registration_date: company.registration_date,
                proof_of_registration_file: company.proof_of_registration_file,
                logo_file: company.logo_file,
                creation_time: test_ts(),
                creator: node_id_test(),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            keys,
            test_ts() - 10,
        )
        .expect("could not create block")
    }

    fn add_creator_identity_proof_block(mut chain: CompanyBlockchain) -> CompanyBlockchain {
        let (proof, mut data) = signed_identity_proof_test();
        data.company_node_id = Some(node_id_test());
        let identity_proof_block = CompanyBlock::create_block_for_identity_proof(
            node_id_test(),
            chain.get_latest_block(),
            &CompanyIdentityProofBlockData {
                proof,
                data,
                reference_block: Some(chain.get_latest_block().id()),
            },
            &BcrKeys::from_private_key(&private_key_test()),
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts() - 9,
        )
        .unwrap();
        assert!(chain.try_add_block(identity_proof_block));
        assert!(chain.is_chain_valid());
        chain
    }

    fn add_invite_signatory_block(mut chain: CompanyBlockchain) -> CompanyBlockchain {
        let block = CompanyBlock::create_block_for_invite_signatory(
            node_id_test(),
            chain.get_latest_block(),
            &CompanyInviteSignatoryBlockData {
                invitee: node_id_test_another(),
                inviter: node_id_test(),
                t: SignatoryType::Solo,
            },
            &BcrKeys::from_private_key(&private_key_test()),
            &BcrKeys::from_private_key(&private_key_test()),
            &node_id_test().pub_key(),
            test_ts() - 8,
        )
        .unwrap();
        assert!(chain.try_add_block(block));
        assert!(chain.is_chain_valid());
        chain
    }

    pub fn get_company_update_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanyUpdateBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_update(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            test_ts(),
        )
        .expect("could not create block")
    }

    fn get_company_invite_signatory_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanyInviteSignatoryBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_invite_signatory(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            &data.invitee.pub_key(),
            test_ts(),
        )
        .expect("could not create block")
    }

    fn get_company_accept_invite_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanySignatoryAcceptInviteBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_accept_signatory_invite(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            test_ts(),
        )
        .expect("could not create block")
    }

    fn get_company_reject_invite_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanySignatoryRejectInviteBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_reject_signatory_invite(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            test_ts(),
        )
        .expect("could not create block")
    }

    fn get_company_remove_signatory_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanyRemoveSignatoryBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_remove_signatory(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            test_ts(),
        )
        .expect("could not create block")
    }

    fn get_company_identity_proof_block(
        node_id: NodeId,
        previous_block: &CompanyBlock,
        keys: &BcrKeys,
        company_keys: &BcrKeys,
        data: &CompanyIdentityProofBlockData,
    ) -> CompanyBlock {
        CompanyBlock::create_block_for_identity_proof(
            node_id,
            previous_block,
            data,
            keys,
            company_keys,
            test_ts(),
        )
        .expect("could not create block")
    }

    #[tokio::test]
    async fn test_process_company_block_effects_does_not_persist_block() {
        // Verify that process_company_block_effects updates company data
        // but does NOT call blockchain_store.add_block
        let (mut chain_store, mut store, _, _, _, mut identity_store, _, _, _, _) = create_mocks();

        let (_, (company, _)) = get_company_data();
        let identity_full = get_baseline_identity();
        let identity_node_id = identity_full.identity.node_id.clone();
        let identity_node_id_for_closure = identity_node_id.clone();

        let update_data = CompanyBlockPayload::Update(update_company_block_with_name(Some(
            Name::new("Updated Company").unwrap(),
        )));

        // CRITICAL: add_block should NEVER be called during side effect processing
        chain_store.expect_add_block().times(0);

        // Company CAN be updated
        store.expect_update().times(1).returning(|_, _| Ok(()));

        // Identity store is needed for getting current identity
        identity_store
            .expect_get_current_identity()
            .returning(move || {
                Ok(ActiveIdentityState {
                    personal: identity_node_id_for_closure.clone(),
                    company: None,
                })
            });

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(MockFileReferenceStore::new()),
            Arc::new(identity_store),
            Arc::new(MockNotificationStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNotificationHandlerApi::new()),
            Arc::new(MockPushApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockContactStore::new()),
            bitcoin::Network::Testnet,
        );

        let mut test_company = company.clone();

        // Call the side effect processing method directly
        let result = handler
            .process_company_block_effects(
                &company.id,
                &mut test_company,
                update_data,
                &identity_node_id,
                test_ts(),
            )
            .await;

        // Should succeed without persisting block
        assert!(result.is_ok());
        // Test passes if add_block was never called (mock verifies this)
    }

    #[tokio::test]
    async fn test_process_company_block_effects_anchors_inbound_files() {
        let mut chain_store = MockCompanyChainStore::new();
        let mut store = MockCompanyStore::new();
        let mut file_reference_store = MockFileReferenceStore::new();
        let company_id = node_id_test();
        let identity_node_id = node_id_test_another();
        let (_, (company, _)) = get_company_data();

        chain_store.expect_add_block().times(0);
        store.expect_update().times(1).returning(|_, _| Ok(()));

        let logo_file = File {
            name: Name::new("logo.png").unwrap(),
            hash: Sha256Hash::new("company_logo_file_hash_123456789012"),
            nostr_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .parse()
                .unwrap(),
        };
        let expected_hash = logo_file.hash.clone();
        let expected_name = logo_file.name.clone();

        file_reference_store
            .expect_upsert()
            .withf(
                move |hash: &Sha256Hash,
                      _nostr_hash: &bitcoin::hashes::sha256::Hash,
                      name: &Option<Name>,
                      server_urls: &Vec<url::Url>,
                      is_important: &Option<bool>,
                      context: &Vec<FileReferenceContext>| {
                    *hash == expected_hash
                        && *name == Some(expected_name.clone())
                        && server_urls.is_empty()
                        && *is_important == Some(true)
                        && context
                            == &vec![FileReferenceContext::Company {
                                company_id: company_id.to_string(),
                                field: "logo_file".to_string(),
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

        let handler = CompanyChainEventProcessor::new(
            Arc::new(chain_store),
            Arc::new(store),
            Arc::new(file_reference_store),
            Arc::new(MockIdentityStore::new()),
            Arc::new(MockNotificationStore::new()),
            Arc::new(MockNostrContactProcessorApi::new()),
            Arc::new(MockNotificationHandlerApi::new()),
            Arc::new(MockPushApi::new()),
            Arc::new(MockNostrChainEventStore::new()),
            Arc::new(MockNotificationJsonTransport::new()),
            Arc::new(MockContactStore::new()),
            bitcoin::Network::Testnet,
        );

        let mut test_company = company.clone();
        let update_data = CompanyBlockPayload::Update(CompanyUpdateBlockData {
            name: None,
            email: None,
            country: None,
            city: None,
            zip: EditOptionalFieldMode::Ignore,
            address: None,
            country_of_registration: EditOptionalFieldMode::Ignore,
            city_of_registration: EditOptionalFieldMode::Ignore,
            registration_number: EditOptionalFieldMode::Ignore,
            registration_date: EditOptionalFieldMode::Ignore,
            logo_file: EditOptionalFieldMode::Set(logo_file),
            proof_of_registration_file: EditOptionalFieldMode::Ignore,
        });

        let result = handler
            .process_company_block_effects(
                &company.id,
                &mut test_company,
                update_data,
                &identity_node_id,
                test_ts(),
            )
            .await;

        assert!(result.is_ok());
    }

    fn create_mocks() -> (
        MockCompanyChainStore,
        MockCompanyStore,
        MockNotificationStore,
        MockNostrContactProcessorApi,
        MockNotificationHandlerApi,
        MockIdentityStore,
        MockNostrChainEventStore,
        MockNotificationJsonTransport,
        MockPushApi,
        MockContactStore,
    ) {
        (
            MockCompanyChainStore::new(),
            MockCompanyStore::new(),
            MockNotificationStore::new(),
            MockNostrContactProcessorApi::new(),
            MockNotificationHandlerApi::new(),
            MockIdentityStore::new(),
            MockNostrChainEventStore::new(),
            MockNotificationJsonTransport::new(),
            MockPushApi::new(),
            MockContactStore::new(),
        )
    }
}
