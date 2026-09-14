use super::error::Error;
use super::{BillAction, BillServiceApi, Result};
use crate::external;
use crate::external::bitcoin::BitcoinClientApi;
use crate::external::court::CourtClientApi;
use crate::external::file_storage::{self, FileStorageClientApi};
use crate::external::mint::{MintClientApi, QuoteStatusReply, ResolveMintOffer};
use crate::get_config;
use crate::service::file_server_service::{
    configured_blossom_servers, download_file_with_fallback, upload_to_blossom_servers_with_server,
};
use crate::service::transport_service::TransportServiceApi;
use crate::util::{validate_bill_id_network, validate_node_id_network};
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::application::bill::{
    AddressDerivationMetadataForPaymentRequest, BillCombinedBitcoinKey, BillRole, BillsBalance,
    BillsBalanceOverview, BillsFilterRole, BitcreditBillResult, Endorsement,
    LightBitcreditBillResult, PastPaymentResult, SweepEstimate, SweepResult,
};
use bcr_ebill_core::application::company::Company;
use bcr_ebill_core::application::contact::{Contact, LightBillParticipant};
use bcr_ebill_core::application::identity::{Identity, IdentityWithAll};
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::blockchain::Block;
use bcr_ebill_core::protocol::blockchain::bill::block::{
    BillIdentParticipantBlockData, BillOfferToSellBlockData, BillParticipantBlockData,
    BillRequestRecourseBlockData, BillRequestToPayBlockData, ContactType,
};
use bcr_ebill_core::protocol::blockchain::bill::chain::BillBlockPlaintextWrapper;
use bcr_ebill_core::protocol::blockchain::bill::{
    BillBlockchain, BillHistory, BillIssueData, BillOpCode, BillValidateActionData,
    BillValidationActionMode, BitcreditBill, create_bill_to_share_with_external_party,
    participant::{BillAnonParticipant, BillIdentParticipant, BillParticipant, PastEndorsee},
};
use bcr_ebill_core::protocol::blockchain::{Blockchain, identity::IdentityType};
use bcr_ebill_core::protocol::crypto::{self, BcrKeys};
use bcr_ebill_core::protocol::event::ActionType;
use bcr_ebill_core::protocol::mint::{MintRequest, MintRequestState, MintRequestStatus};
use bcr_ebill_core::protocol::{BitcoinAddress, Email};
use bcr_ebill_core::protocol::{BlockId, PublicKey};
use bcr_ebill_core::protocol::{Currency, Sum};
use bcr_ebill_core::{
    application::ServiceTraitBounds, application::ValidationError, protocol::File,
    protocol::Validate,
};
use bcr_ebill_persistence::ContactStoreApi;
use bcr_ebill_persistence::FileReferenceStoreApi;
use bcr_ebill_persistence::bill::{BillChainStoreApi, BillStoreApi};
use bcr_ebill_persistence::company::{CompanyChainStoreApi, CompanyStoreApi};
use bcr_ebill_persistence::file_upload::FileUploadStoreApi;
use bcr_ebill_persistence::identity::{IdentityChainStoreApi, IdentityStoreApi};
use bcr_ebill_persistence::mint::MintStoreApi;
use bcr_ebill_persistence::nostr::NostrContactStoreApi;
use bitcoin::secp256k1::SecretKey;
use log::{debug, error, info};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// The bill service is responsible for all bill-related logic and for syncing them with the
/// network
#[derive(Clone)]
pub struct BillService {
    pub store: Arc<dyn BillStoreApi>,
    pub blockchain_store: Arc<dyn BillChainStoreApi>,
    pub identity_store: Arc<dyn IdentityStoreApi>,
    pub file_upload_store: Arc<dyn FileUploadStoreApi>,
    pub file_upload_client: Arc<dyn FileStorageClientApi>,
    pub file_reference_store: Arc<dyn FileReferenceStoreApi>,
    pub bitcoin_client: Arc<dyn BitcoinClientApi>,
    pub transport_service: Arc<dyn TransportServiceApi>,
    pub identity_blockchain_store: Arc<dyn IdentityChainStoreApi>,
    pub company_blockchain_store: Arc<dyn CompanyChainStoreApi>,
    pub contact_store: Arc<dyn ContactStoreApi>,
    pub company_store: Arc<dyn CompanyStoreApi>,
    pub mint_store: Arc<dyn MintStoreApi>,
    pub mint_client: Arc<dyn MintClientApi>,
    pub court_client: Arc<dyn CourtClientApi>,
    pub nostr_contact_store: Arc<dyn NostrContactStoreApi>,
}
impl ServiceTraitBounds for BillService {}

impl BillService {
    pub fn new(
        store: Arc<dyn BillStoreApi>,
        blockchain_store: Arc<dyn BillChainStoreApi>,
        identity_store: Arc<dyn IdentityStoreApi>,
        file_upload_store: Arc<dyn FileUploadStoreApi>,
        file_upload_client: Arc<dyn FileStorageClientApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        bitcoin_client: Arc<dyn BitcoinClientApi>,
        transport_service: Arc<dyn TransportServiceApi>,
        identity_blockchain_store: Arc<dyn IdentityChainStoreApi>,
        company_blockchain_store: Arc<dyn CompanyChainStoreApi>,
        contact_store: Arc<dyn ContactStoreApi>,
        company_store: Arc<dyn CompanyStoreApi>,
        mint_store: Arc<dyn MintStoreApi>,
        mint_client: Arc<dyn MintClientApi>,
        court_client: Arc<dyn CourtClientApi>,
        nostr_contact_store: Arc<dyn NostrContactStoreApi>,
    ) -> Self {
        Self {
            store,
            blockchain_store,
            identity_store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            bitcoin_client,
            transport_service,
            identity_blockchain_store,
            company_blockchain_store,
            contact_store,
            company_store,
            mint_store,
            mint_client,
            court_client,
            nostr_contact_store,
        }
    }

    /// Recalculates the full bill and updates it in the cache
    pub(super) async fn recalculate_and_persist_bill(
        &self,
        bill_id: &BillId,
        chain: &BillBlockchain,
        bill_keys: &BcrKeys,
        local_identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<()> {
        let calculated_bill = self
            .calculate_full_bill(
                chain,
                bill_keys,
                local_identity,
                caller_public_data,
                caller_keys,
                current_timestamp,
            )
            .await?;
        self.store
            .save_bill_to_cache(bill_id, &caller_public_data.node_id(), &calculated_bill)
            .await?;
        Ok(())
    }

    pub(super) async fn extend_bill_chain_participant_data_from_contacts_or_identity(
        &self,
        chain_identity: BillParticipantBlockData,
        identity: &Identity,
        contacts: &HashMap<NodeId, Contact>,
    ) -> BillParticipant {
        match chain_identity {
            BillParticipantBlockData::Ident(data) => BillParticipant::Ident(
                self.extend_bill_chain_identity_data_from_contacts_or_identity(
                    data, identity, contacts,
                )
                .await,
            ),
            BillParticipantBlockData::Anon(data) => {
                let (_, nostr_relays) = self
                    .get_email_and_nostr_relay(&data.node_id, ContactType::Anon, identity, contacts)
                    .await;
                BillParticipant::Anon(BillAnonParticipant {
                    node_id: data.node_id,
                    nostr_relays,
                })
            }
        }
    }

    /// If it's our identity, we take the fields from there, otherwise we check contacts,
    /// companies, or leave them empty
    pub(super) async fn extend_bill_chain_identity_data_from_contacts_or_identity(
        &self,
        chain_identity: BillIdentParticipantBlockData,
        identity: &Identity,
        contacts: &HashMap<NodeId, Contact>,
    ) -> BillIdentParticipant {
        let (email, nostr_relays) = self
            .get_email_and_nostr_relay(
                &chain_identity.node_id,
                chain_identity.t.clone(),
                identity,
                contacts,
            )
            .await;
        BillIdentParticipant {
            t: chain_identity.t,
            node_id: chain_identity.node_id,
            name: chain_identity.name,
            postal_address: chain_identity.postal_address,
            email,
            nostr_relays,
        }
    }

    async fn get_email_and_nostr_relay(
        &self,
        node_id: &NodeId,
        t: ContactType,
        identity: &Identity,
        contacts: &HashMap<NodeId, Contact>,
    ) -> (Option<Email>, Vec<url::Url>) {
        match node_id {
            v if *v == identity.node_id => (identity.email.clone(), identity.nostr_relays.clone()),
            other_node_id => {
                if let Some(contact) = contacts.get(other_node_id) {
                    (contact.email.clone(), contact.nostr_relays.clone())
                } else if t == ContactType::Company {
                    if let Ok(company) = self.company_store.get(other_node_id).await {
                        (
                            Some(company.email.clone()),
                            identity.nostr_relays.clone(), // if it's a local company, we take our relay
                        )
                    } else {
                        (None, vec![])
                    }
                } else if let Ok(Some(nostr_contact)) =
                    self.nostr_contact_store.by_node_id(node_id).await
                {
                    (None, nostr_contact.relays)
                } else {
                    (None, vec![])
                }
            }
        }
    }

    async fn check_bill_timeouts(&self, bill_id: &BillId, now: Timestamp) -> Result<()> {
        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let contacts = self.contact_store.get_map().await?;
        let mut recoursee = None;

        let latest_block = chain.get_latest_block();
        if let Some(action) = match latest_block.op_code {
            BillOpCode::RequestToPay
                if chain
                    .is_req_to_pay_block_payment_expired(latest_block, &bill_keys, now)
                    .map_err(|e| Error::Protocol(e.into()))?
                    .0 =>
            {
                Some(ActionType::PayBill)
            }
            BillOpCode::OfferToSell
                if chain
                    .is_offer_to_sell_block_payment_expired(latest_block, &bill_keys, now)
                    .map_err(|e| Error::Protocol(e.into()))?
                    .0 =>
            {
                Some(ActionType::PayBill)
            }

            BillOpCode::RequestToAccept
                if chain
                    .is_req_to_accept_block_expired(latest_block, &bill_keys, now)
                    .map_err(|e| Error::Protocol(e.into()))?
                    .0 =>
            {
                Some(ActionType::AcceptBill)
            }
            BillOpCode::RequestRecourse
                if chain
                    .is_req_to_recourse_block_payment_expired(latest_block, &bill_keys, now)
                    .map_err(|e| Error::Protocol(e.into()))?
                    .0 =>
            {
                if let Ok(recourse_block) =
                    latest_block.get_decrypted_block::<BillRequestRecourseBlockData>(&bill_keys)
                {
                    recoursee = Some(recourse_block.recoursee.node_id());
                }
                Some(ActionType::RecourseBill)
            }
            _ => None,
        } {
            if let Err(e) = self.store.invalidate_bill_in_cache(bill_id).await {
                error!("Failed to invalidate cache for {bill_id} before timeout notification: {e}");
            }

            // did we already send the notification
            let sent = self
                .transport_service
                .notification_transport()
                .check_bill_notification_sent(
                    bill_id,
                    chain.block_height() as i32,
                    action.to_owned(),
                )
                .await?;

            if !sent {
                let identity = self.identity_store.get().await?;
                let current_identity = match identity.t {
                    IdentityType::Ident => BillIdentParticipant::new(identity.clone())
                        .map(BillParticipant::Ident)
                        .ok(),
                    IdentityType::Anon => Some(BillParticipant::Anon(BillAnonParticipant::new(
                        identity.clone(),
                    ))),
                };
                let participants = chain
                    .get_all_nodes_from_bill(&bill_keys)
                    .map_err(|e| Error::Protocol(e.into()))?;
                let mut recipient_options: Vec<Option<BillParticipant>> = vec![current_identity];
                let bill = self
                    .get_last_version_bill(&chain, &bill_keys, &identity, &contacts)
                    .await?;

                let holder = bill.endorsee.as_ref().unwrap_or(&bill.payee).node_id();
                let drawee = &bill.drawee.node_id;

                for node_id in participants {
                    if let Some(contact) = self.contact_store.get(&node_id).await? {
                        recipient_options.push(Some(contact.try_into()?));
                    }
                }

                let recipients = recipient_options
                    .into_iter()
                    .flatten()
                    .collect::<Vec<BillParticipant>>();

                self.transport_service
                    .notification_transport()
                    .send_request_to_action_timed_out_event(
                        &identity.node_id,
                        bill_id,
                        Some(bill.sum),
                        action.to_owned(),
                        recipients,
                        &holder,
                        drawee,
                        &recoursee,
                    )
                    .await?;

                // remember we have sent the notification
                self.transport_service
                    .notification_transport()
                    .mark_bill_notification_sent(bill_id, chain.block_height() as i32, action)
                    .await?;
            }
        }
        Ok(())
    }

    /// Checks a given mint quote and updates the bill mint state, if there was a change
    pub(super) async fn check_mint_quote_and_update_bill_mint_state(
        &self,
        mint_request: &MintRequest,
    ) -> Result<()> {
        debug!(
            "Checking mint request for quote {}",
            mint_request.mint_request_id
        );
        let mint_cfg = &get_config().mint_config;
        // for now, we only support the default mint
        if mint_request.mint_node_id != mint_cfg.default_mint_node_id {
            return Ok(());
        }

        match mint_request.status {
            // If it's minting enabled, get the offer and, if it's not finished (i.e. has no proofs), attempt to get keyset and mint
            MintRequestStatus::MintingEnabled => {
                if let Ok(Some(offer)) = self
                    .mint_store
                    .get_offer(&mint_request.mint_request_id)
                    .await
                {
                    // only if there are no proofs yet and proofs are not spent
                    if offer.proofs.is_none() && !offer.proofs_spent {
                        debug!(
                            "Checking for keyset info for {}",
                            mint_request.mint_request_id
                        );

                        // Quote is MintingEnabled - attempt to mint
                        // check keyset and try to mint and create tokens and persist
                        match self
                            .mint_client
                            .get_keyset_info(&mint_cfg.default_mint_url, &offer.keyset_id)
                            .await
                        {
                            // keyset info is available
                            Ok(keyset) => {
                                // fetch private key for requester
                                let private_key = match self.identity_store.get_full().await {
                                    Ok(identity) => {
                                        // check if requester is identity
                                        if identity.identity.node_id
                                            == mint_request.requester_node_id
                                        {
                                            identity.key_pair.get_private_key()
                                        } else {
                                            // check if requester is a company
                                            let local_companies: HashMap<
                                                NodeId,
                                                (Company, BcrKeys),
                                            > = self.company_store.get_all().await?;
                                            if let Some(requester_company) =
                                                local_companies.get(&mint_request.requester_node_id)
                                            {
                                                requester_company.1.get_private_key()
                                            } else {
                                                // requester is neither identity, nor company
                                                log::warn!(
                                                    "Requester for {} is not a local identity, or company",
                                                    mint_request.mint_request_id
                                                );
                                                return Ok(());
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        return Err(e.into());
                                    }
                                };
                                debug!(
                                    "Keyset found and minting for {}",
                                    mint_request.mint_request_id
                                );
                                // generate blinds
                                let (blinded_messages, secrets, rs) =
                                    external::mint::generate_blinds(&keyset, offer.discounted_sum)?;
                                // persist recovery data in case something goes wrong after minting
                                // getting here a second time for this offer will fail, which is OK
                                // since it means that either minting, or proof creation failed and we can't
                                // detect which one reliably
                                // with the secrets and rs, we can re-create the blinded messages and get
                                // blinded signatures from the mint using the mint, OR the recovery endpoint
                                if let Err(e) = self
                                    .mint_store
                                    .add_recovery_data_to_offer(
                                        &mint_request.mint_request_id,
                                        &secrets
                                            .iter()
                                            .map(|s| s.to_string())
                                            .collect::<Vec<String>>(),
                                        &rs.iter().map(|r| r.to_string()).collect::<Vec<String>>(),
                                    )
                                    .await
                                {
                                    error!(
                                        "Couldn't set recovery data for quote {}: {e}",
                                        mint_request.mint_request_id
                                    );
                                }

                                // mint and generate proofs
                                let proofs = self
                                    .mint_client
                                    .mint(
                                        &mint_request.bill_id,
                                        &mint_cfg.default_mint_url,
                                        keyset,
                                        &mint_request.mint_request_id,
                                        &private_key,
                                        blinded_messages,
                                        secrets,
                                        rs,
                                    )
                                    .await?;
                                // store proofs on the offer
                                self.mint_store
                                    .add_proofs_to_offer(&mint_request.mint_request_id, &proofs)
                                    .await?;
                            }
                            Err(_) => {
                                info!("No keyset available for {}", mint_request.mint_request_id);
                            }
                        };
                    } else if offer.proofs.is_some() && !offer.proofs_spent {
                        debug!(
                            "Checking if proofs for {} are spent",
                            mint_request.mint_request_id
                        );
                        if let Some(proofs) = offer.proofs {
                            match self
                                .mint_client
                                .check_if_proofs_are_spent(
                                    &mint_cfg.default_mint_url,
                                    &proofs,
                                    &offer.keyset_id,
                                )
                                .await
                            {
                                Ok(spent) => {
                                    if spent {
                                        debug!(
                                            "Proofs for {} are spent - updating",
                                            mint_request.mint_request_id
                                        );
                                        self.mint_store
                                            .set_proofs_to_spent_for_offer(
                                                &mint_request.mint_request_id,
                                            )
                                            .await?;
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        "Could not check if proofs are spent for {}: {e}",
                                        mint_request.mint_request_id
                                    );
                                }
                            }
                        }
                    } else {
                        // proofs spent - nothing to do
                    }
                }
            }
            MintRequestStatus::Pending
            | MintRequestStatus::Offered
            | MintRequestStatus::Accepted => {
                let updated_status = self
                    .mint_client
                    .lookup_quote_for_mint(
                        &mint_cfg.default_mint_url,
                        &mint_request.mint_request_id,
                    )
                    .await?;
                // only update, if changed
                match updated_status {
                    QuoteStatusReply::Pending => {
                        // it's already pending, or offered, or accepted and can't go from Accepted to Offered to Pending - nothing to do
                    }
                    QuoteStatusReply::FailedEbillValidation { .. } => {
                        // it's already endorsed and failed validation - nothing to do
                    }
                    QuoteStatusReply::Offered {
                        keyset_id,
                        expiration_date,
                        discounted,
                    } => {
                        // if it's not already offered, set to offered and store the offer
                        if !matches!(mint_request.status, MintRequestStatus::Offered) {
                            // Update the request
                            self.mint_store
                                .update_request(
                                    &mint_request.mint_request_id,
                                    &MintRequestStatus::Offered,
                                )
                                .await?;
                            // Store the offer
                            self.mint_store
                                .add_offer(
                                    &mint_request.mint_request_id,
                                    &keyset_id.to_string(),
                                    Timestamp::from(expiration_date),
                                    discounted.into(),
                                )
                                .await?;
                        }
                    }
                    QuoteStatusReply::Accepted { .. } => {
                        // if it's not already accepted, set to accepted
                        if !matches!(mint_request.status, MintRequestStatus::Accepted) {
                            // Update the request
                            self.mint_store
                                .update_request(
                                    &mint_request.mint_request_id,
                                    &MintRequestStatus::Accepted,
                                )
                                .await?;
                        }
                    }
                    QuoteStatusReply::Denied { tstamp } => {
                        // checked below, that it's not denied
                        self.mint_store
                            .update_request(
                                &mint_request.mint_request_id,
                                &MintRequestStatus::Denied {
                                    timestamp: Timestamp::from(tstamp),
                                },
                            )
                            .await?;
                    }
                    QuoteStatusReply::Expired { tstamp } => {
                        // checked below, that it's not expired
                        self.mint_store
                            .update_request(
                                &mint_request.mint_request_id,
                                &MintRequestStatus::Expired {
                                    timestamp: Timestamp::from(tstamp),
                                },
                            )
                            .await?;
                    }
                    QuoteStatusReply::Cancelled { tstamp } => {
                        // checked below, that it's not cancelled
                        self.mint_store
                            .update_request(
                                &mint_request.mint_request_id,
                                &MintRequestStatus::Cancelled {
                                    timestamp: Timestamp::from(tstamp),
                                },
                            )
                            .await?;
                    }
                    QuoteStatusReply::Rejected { tstamp } => {
                        // checked below, that it's not rejected
                        self.mint_store
                            .update_request(
                                &mint_request.mint_request_id,
                                &MintRequestStatus::Rejected {
                                    timestamp: Timestamp::from(tstamp),
                                },
                            )
                            .await?;
                    }
                    QuoteStatusReply::MintingEnabled { .. } => {
                        // checked above, that it's not minting enabled
                        self.mint_store
                            .update_request(
                                &mint_request.mint_request_id,
                                &MintRequestStatus::MintingEnabled,
                            )
                            .await?;
                    }
                };
            }
            // Cancelled, Rejected, Expired, Denied
            _ => {
                // Req to mint is finished - nothing to do
            }
        };
        Ok(())
    }

    async fn get_req_to_mint_for_node_id(
        &self,
        mint_request_id: &Uuid,
        current_identity_node_id: &NodeId,
    ) -> Result<MintRequest> {
        match self.mint_store.get_request(mint_request_id).await {
            Ok(Some(req)) => {
                // only if we're the requester
                if req.requester_node_id == *current_identity_node_id {
                    let mint_cfg = &get_config().mint_config;
                    // only for the default mint for now
                    if req.mint_node_id == mint_cfg.default_mint_node_id {
                        Ok(req.to_owned())
                    } else {
                        Err(Error::NotFound)
                    }
                } else {
                    Err(Error::NotFound)
                }
            }
            Ok(None) => Err(Error::NotFound),
            Err(e) => Err(e.into()),
        }
    }

    async fn reject_mint_offers_except_accepted_one_for_bill(
        &self,
        accepted_mint_request_id: &Uuid,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<()> {
        let requests = self
            .mint_store
            .get_requests_for_bill(current_identity_node_id, bill_id)
            .await?;
        for req in requests {
            // don't reject the accepted one and only reject offered ones
            if &req.mint_request_id != accepted_mint_request_id
                && matches!(req.status, MintRequestStatus::Offered)
                && let Err(e) = self
                    .mint_client
                    .resolve_quote_for_mint(
                        &get_config().mint_config.default_mint_url,
                        &req.mint_request_id,
                        ResolveMintOffer::Reject,
                    )
                    .await
            {
                error!(
                    "Could not reject quote for mint {}: {e}",
                    req.mint_request_id
                )
            }
        }
        Ok(())
    }

    async fn get_participant_for_mint(
        &self,
        mint_node_id: &NodeId,
        identity: &Identity,
    ) -> BillParticipant {
        let relays = match self
            .transport_service
            .contact_transport()
            .resolve_contact(mint_node_id)
            .await
        {
            Ok(Some(nostr_contact_data)) => nostr_contact_data
                .relays
                .iter()
                .map(|url| url.to_owned().into())
                .collect::<Vec<url::Url>>(),
            _ => {
                // fallback to own relays
                identity.nostr_relays.clone()
            }
        };
        BillParticipant::Anon(BillAnonParticipant {
            node_id: mint_node_id.to_owned(),
            nostr_relays: relays,
        })
    }

    pub(super) async fn upload_bill_files_for_node_id(
        &self,
        bill_id: &BillId,
        bill_private_key: &SecretKey,
        receiver_public_key: &PublicKey,
        signer: &BcrKeys,
        files: &[File],
    ) -> Result<Vec<url::Url>> {
        if files.is_empty() {
            return Ok(vec![]);
        }
        let blossom_servers = configured_blossom_servers(&get_config().nostr_config);

        let mut file_urls = Vec::with_capacity(files.len());
        for file in files {
            let decrypted_file = self
                .open_and_decrypt_attached_file(bill_id, file, bill_private_key)
                .await?;
            let encrypted_file = crypto::encrypt_ecies(&decrypted_file, receiver_public_key)?;

            let (uploaded_server, uploaded_hash, _confirmed_servers) =
                upload_to_blossom_servers_with_server(
                    self.file_upload_client.as_ref(),
                    &blossom_servers,
                    encrypted_file,
                    signer,
                )
                .await
                .map_err(Error::from)?;

            file_urls.push(file_storage::to_url(
                &uploaded_server,
                &uploaded_hash.to_string(),
            )?);
        }

        Ok(file_urls)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl BillServiceApi for BillService {
    async fn get_bill_balances(
        &self,
        _currency: &Currency,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<BillsBalanceOverview> {
        let current_identity_node_id = caller_public_data.node_id();
        // TODO (currency): convert between currencies based on given currency
        validate_node_id_network(&current_identity_node_id)?;
        let bills = self.get_bills(caller_public_data, caller_keys).await?;

        let mut payer_sum = 0;
        let mut payee_sum = 0;
        let mut contingent_sum = 0;

        for bill in bills {
            if let Some(bill_role) = bill.get_bill_role_for_node_id(&current_identity_node_id) {
                let sum = bill.data.sum;
                match bill_role {
                    BillRole::Payee => payee_sum += sum.as_sat(),
                    BillRole::Payer => payer_sum += sum.as_sat(),
                    BillRole::Contingent => {
                        // if we're in the guarantee chain, but only anonymously, we don't count it
                        let endorsements = bill.participants.endorsements;
                        let mut in_guarantee_chain_as_non_anon = false;
                        for endorsement in endorsements.iter() {
                            let holder = &endorsement.pay_to_the_order_of;
                            // we're in the chain as non-anon
                            if holder.node_id() == current_identity_node_id
                                && matches!(holder, LightBillParticipant::Ident(_))
                            {
                                in_guarantee_chain_as_non_anon = true;
                                break;
                            }
                        }

                        // If we're contingent, but not in endorsements, we could be payee
                        if !in_guarantee_chain_as_non_anon
                            && bill.participants.payee.node_id() == current_identity_node_id
                            && matches!(bill.participants.payee, BillParticipant::Ident(_))
                        {
                            in_guarantee_chain_as_non_anon = true;
                        }

                        if in_guarantee_chain_as_non_anon
                            || bill.participants.drawer.node_id == current_identity_node_id
                        {
                            contingent_sum += sum.as_sat()
                        }
                    }
                };
            }
        }

        Ok(BillsBalanceOverview {
            payee: BillsBalance {
                sum: Sum::new_sat_zero_allowed(payee_sum),
            },
            payer: BillsBalance {
                sum: Sum::new_sat_zero_allowed(payer_sum),
            },
            contingent: BillsBalance {
                sum: Sum::new_sat_zero_allowed(contingent_sum),
            },
        })
    }

    async fn search_bills(
        &self,
        _currency: &Currency,
        search_term: &Option<String>,
        date_range_from: Option<Timestamp>,
        date_range_to: Option<Timestamp>,
        role: &BillsFilterRole,
        participants: &[NodeId],
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<LightBitcreditBillResult>> {
        debug!(
            "searching bills with {search_term:?} from {date_range_from:?} to {date_range_to:?} and {role:?}"
        );
        let current_identity_node_id = caller_public_data.node_id();
        validate_node_id_network(&current_identity_node_id)?;
        let bills = self.get_bills(caller_public_data, caller_keys).await?;
        let mut result = vec![];

        let filter_set: HashSet<NodeId> = participants.iter().cloned().collect();
        // for now we do the search here - with the quick-fetch table, we can search in surrealDB
        // directly
        for bill in bills {
            // if the bill wasn't issued between from and to, we kick them out
            let issue_date_ts = bill.data.issue_date.to_timestamp();
            if let Some(from) = date_range_from
                && from > issue_date_ts
            {
                continue;
            }
            if let Some(to) = date_range_to
                && to < issue_date_ts
            {
                continue;
            }

            let bill_role = match bill.get_bill_role_for_node_id(&current_identity_node_id) {
                Some(bill_role) => bill_role,
                None => continue, // node is not in bill - don't add
            };

            match role {
                BillsFilterRole::All => (), // we take all
                BillsFilterRole::Payer => {
                    if bill_role != BillRole::Payer {
                        // payer selected, but node not payer
                        continue;
                    }
                }
                BillsFilterRole::Payee => {
                    if bill_role != BillRole::Payee {
                        // payee selected, but node not payee
                        continue;
                    }
                }
                BillsFilterRole::Contingent => {
                    if bill_role != BillRole::Contingent {
                        // contingent selected, but node not
                        // contingent
                        continue;
                    }
                }
            };

            if let Some(st) = search_term
                && !bill.search_bill_for_search_term(st)
            {
                continue;
            }

            // if we search for participants and not all of the given participants are part of the bill, continue
            if !filter_set.is_empty() {
                let bill_set: HashSet<NodeId> = bill
                    .participants
                    .all_participant_node_ids
                    .iter()
                    .cloned()
                    .collect();
                // every node id in filter is contained in the bill participants
                if !filter_set.is_subset(&bill_set) {
                    continue;
                }
            }

            result.push(bill.into());
        }

        Ok(result)
    }

    async fn get_bills(
        &self,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<BitcreditBillResult>> {
        validate_node_id_network(&caller_public_data.node_id())?;
        let bill_ids = self.store.get_ids().await?;
        let identity = self.identity_store.get().await?;
        let current_timestamp = Timestamp::now();

        // fetch contacts to get current contact data for participants
        let contacts = self.contact_store.get_map().await?;

        let mut bills = self
            .store
            .get_bills_from_cache(&bill_ids, &caller_public_data.node_id())
            .await?;
        // extend identities for cached bills
        for bill in bills.iter_mut() {
            self.extend_bill_identities_from_contacts_or_identity(bill, &identity, &contacts)
                .await;

            // check requests for being expired - if an active req to
            // accept/pay/recourse/sell is expired, we need to recalculate the bill
            if self.check_requests_for_expiration(bill, current_timestamp)? {
                debug!(
                    "Bill cache hit, but needs to recalculate because of request deadline {} - recalculating",
                    bill.id
                );
                *bill = self
                    .recalculate_and_cache_bill(
                        &bill.id,
                        &identity,
                        caller_public_data,
                        caller_keys,
                        current_timestamp,
                    )
                    .await?;
            }
        }

        for bill_id in bill_ids.iter() {
            // if bill was not in cache - recalculate and cache it
            if !bills.iter().any(|bill| *bill_id == bill.id) {
                debug!("Bill {bill_id} was not in the cache - recalculate");
                let calculated_bill = self
                    .recalculate_and_cache_bill(
                        bill_id,
                        &identity,
                        caller_public_data,
                        caller_keys,
                        current_timestamp,
                    )
                    .await?;
                bills.push(calculated_bill);
            }
        }

        // fetch active notifications for bills
        let active_notifications = self
            .transport_service
            .notification_transport()
            .get_active_bill_notifications(&bill_ids)
            .await;
        for bill in bills.iter_mut() {
            bill.data.active_notification = active_notifications.get(&bill.id).cloned();
        }

        // only return bills where the current node id is a participant
        Ok(bills
            .into_iter()
            .filter(|b| {
                b.participants
                    .all_participant_node_ids
                    .iter()
                    .any(|p| p == &caller_public_data.node_id())
            })
            .collect())
    }

    async fn get_combined_bitcoin_keys_for_bill(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<BillCombinedBitcoinKey>> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;

        // if caller is not part of the bill, they can't access it
        if !chain
            .get_all_nodes_from_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?
            .iter()
            .any(|p| p == &caller_public_data.node_id())
        {
            debug!("caller is not a participant of bill {bill_id}");
            return Err(Error::NotFound);
        }

        // If the caller is the mint holding the bill, there are no btc descriptors available
        // since they use a custom descriptor
        if let Some(mint_holder) = chain
            .holder_is_mint(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?
            && caller_public_data.node_id() == mint_holder.node_id()
        {
            return Ok(vec![]);
        }

        let btc_network = get_config().bitcoin_network();
        let mut res: Vec<BillCombinedBitcoinKey> = Vec::new();
        // Iterate the chain and for each payment request block by the caller, create the descriptor and collect metadata
        for block in chain.blocks().iter() {
            match block.op_code() {
                BillOpCode::RequestToPay => {
                    let block_data: BillRequestToPayBlockData = block
                        .get_decrypted_block(&bill_keys)
                        .map_err(|e| Error::Protocol(e.into()))?;
                    // we are requester
                    if block_data.requester.node_id() == caller_public_data.node_id() {
                        res.push(
                            BillCombinedBitcoinKey::from_block_and_keys(
                                block,
                                &bill_keys,
                                caller_keys,
                                btc_network,
                            )
                            .map_err(Error::Protocol)?,
                        );
                    }
                }
                BillOpCode::OfferToSell => {
                    let block_data: BillOfferToSellBlockData = block
                        .get_decrypted_block(&bill_keys)
                        .map_err(|e| Error::Protocol(e.into()))?;
                    // we are requester
                    if block_data.seller.node_id() == caller_public_data.node_id() {
                        res.push(
                            BillCombinedBitcoinKey::from_block_and_keys(
                                block,
                                &bill_keys,
                                caller_keys,
                                btc_network,
                            )
                            .map_err(Error::Protocol)?,
                        );
                    }
                }
                BillOpCode::RequestRecourse => {
                    let block_data: BillRequestRecourseBlockData = block
                        .get_decrypted_block(&bill_keys)
                        .map_err(|e| Error::Protocol(e.into()))?;
                    // we are requester
                    if block_data.recourser.node_id() == caller_public_data.node_id() {
                        res.push(
                            BillCombinedBitcoinKey::from_block_and_keys(
                                block,
                                &bill_keys,
                                caller_keys,
                                btc_network,
                            )
                            .map_err(Error::Protocol)?,
                        );
                    }
                }
                _ => {} // nothing to do
            };
        }

        return Ok(res);
    }

    async fn get_detail(
        &self,
        bill_id: &BillId,
        identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<BitcreditBillResult> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        let res = self
            .get_full_bill(
                bill_id,
                identity,
                caller_public_data,
                caller_keys,
                current_timestamp,
            )
            .await?;
        // if currently active identity is not part of the bill, we can't access it
        if !res
            .participants
            .all_participant_node_ids
            .iter()
            .any(|p| p == &caller_public_data.node_id())
        {
            return Err(Error::NotFound);
        }
        Ok(res)
    }

    async fn get_bill_keys(&self, bill_id: &BillId) -> Result<BcrKeys> {
        validate_bill_id_network(bill_id)?;
        match self.store.exists(bill_id).await {
            Ok(true) => (),
            _ => {
                return Err(Error::NotFound);
            }
        };
        let keys = self.store.get_keys(bill_id).await?;
        Ok(keys)
    }

    async fn open_and_decrypt_attached_file(
        &self,
        bill_id: &BillId,
        file: &File,
        bill_private_key: &SecretKey,
    ) -> Result<Vec<u8>> {
        debug!("getting file {} for bill with id: {bill_id}", file.name);
        validate_bill_id_network(bill_id)?;
        let file_bytes = download_file_with_fallback(
            self.file_upload_client.as_ref(),
            Some(&self.file_reference_store),
            Some(&self.transport_service),
            &configured_blossom_servers(&get_config().nostr_config),
            &file.hash,
            &file.nostr_hash,
        )
        .await
        .map_err(Error::from)?;
        let decrypted = crypto::decrypt_ecies(&file_bytes, bill_private_key)?;
        let file_hash = Sha256Hash::from_bytes(&decrypted);
        if file_hash == file.hash {
            return Ok(decrypted);
        }
        error!(
            "Hash for bill file {} did not match uploaded file",
            file.name
        );
        Err(super::Error::NotFound)
    }

    async fn issue_new_bill(&self, data: BillIssueData) -> Result<BitcreditBill> {
        self.issue_bill(data).await
    }

    async fn execute_bill_action(
        &self,
        bill_id: &BillId,
        bill_action: BillAction,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<BillBlockchain> {
        debug!("Executing bill action {:?} for bill {bill_id}", bill_action);
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&signer_public_data.node_id())?;
        // fetch data
        let identity = self.identity_store.get_full().await?;
        let contacts = self.contact_store.get_map().await?;
        let mut blockchain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let bill = self
            .get_last_version_bill(&blockchain, &bill_keys, &identity.identity, &contacts)
            .await?;
        let is_paid = self.store.is_paid(bill_id).await?;

        // validate
        BillValidateActionData {
            blockchain: blockchain.clone(),
            drawee_node_id: bill.drawee.node_id.clone(),
            payee_node_id: bill.payee.node_id().clone(),
            endorsee_node_id: bill.endorsee.clone().map(|e| e.node_id()),
            maturity_date: bill.maturity_date.clone(),
            bill_keys: bill_keys.clone(),
            timestamp,
            signer_node_id: signer_public_data.node_id().clone(),
            is_paid,
            mode: BillValidationActionMode::Deep(bill_action.clone()),
        }
        .validate()?;

        // create and sign blocks
        self.create_blocks_for_bill_action(
            &bill,
            &mut blockchain,
            &bill_keys,
            &bill_action,
            &signer_public_data.clone(),
            signer_keys,
            &identity,
            timestamp,
        )
        .await?;

        // Calculate bill and persist it to cache
        self.recalculate_and_persist_bill(
            bill_id,
            &blockchain,
            &bill_keys,
            &identity.identity,
            signer_public_data,
            signer_keys,
            timestamp,
        )
        .await?;

        // notify and propagate blocks
        self.notify_for_block_action(
            &blockchain,
            &bill_keys,
            &bill_action,
            &identity.identity,
            &contacts,
            &signer_public_data.node_id(),
        )
        .await?;

        debug!("Executed bill action {:?} for bill {bill_id}", bill_action);

        Ok(blockchain)
    }

    async fn check_bills_payment(&self) -> Result<()> {
        let identity = self.identity_store.get().await?;
        let bill_ids_waiting_for_payment = self.store.get_bill_ids_waiting_for_payment().await?;

        for bill_id in bill_ids_waiting_for_payment {
            if let Err(e) = self.check_bill_payment(&bill_id, &identity).await {
                error!("Checking bill payment for {bill_id} failed: {e}");
            }
        }
        Ok(())
    }

    async fn check_payment_for_bill(&self, bill_id: &BillId, identity: &Identity) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        let bill_ids_waiting_for_payment = self.store.get_bill_ids_waiting_for_payment().await?;

        if bill_ids_waiting_for_payment.iter().any(|id| id == bill_id) {
            if let Err(e) = self.check_bill_payment(bill_id, identity).await {
                error!("Checking bill payment for {bill_id} failed: {e}");
            }
        } else {
            info!("Bill with id {bill_id} is not waiting for payment - not checking payment");
        }
        Ok(())
    }

    async fn check_bills_offer_to_sell_payment(&self) -> Result<()> {
        let identity = self.identity_store.get_full().await?;
        let bill_ids_waiting_for_offer_to_sell_payment =
            self.store.get_bill_ids_waiting_for_sell_payment().await?;
        let now = Timestamp::now();

        for bill_id in bill_ids_waiting_for_offer_to_sell_payment {
            if let Err(e) = self
                .check_bill_offer_to_sell_payment(&bill_id, &identity, now)
                .await
            {
                error!("Checking offer to sell payment for {bill_id} failed: {e}");
            }
        }
        Ok(())
    }

    async fn check_offer_to_sell_payment_for_bill(
        &self,
        bill_id: &BillId,
        identity: &IdentityWithAll,
    ) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        let bill_ids_waiting_for_offer_to_sell_payment =
            self.store.get_bill_ids_waiting_for_sell_payment().await?;
        let now = Timestamp::now();

        if bill_ids_waiting_for_offer_to_sell_payment
            .iter()
            .any(|id| id == bill_id)
        {
            if let Err(e) = self
                .check_bill_offer_to_sell_payment(bill_id, identity, now)
                .await
            {
                error!("Checking offer to sell payment for {bill_id} failed: {e}");
            }
        } else {
            info!(
                "Bill with id {bill_id} is not waiting for offer to sell payment - not checking payment"
            );
        }
        Ok(())
    }

    async fn check_bills_in_recourse_payment(&self) -> Result<()> {
        let identity = self.identity_store.get_full().await?;
        let bill_ids_waiting_for_recourse_payment = self
            .store
            .get_bill_ids_waiting_for_recourse_payment()
            .await?;
        let now = Timestamp::now();

        for bill_id in bill_ids_waiting_for_recourse_payment {
            if let Err(e) = self
                .check_bill_in_recourse_payment(&bill_id, &identity, now)
                .await
            {
                error!("Checking recourse payment for {bill_id} failed: {e}");
            }
        }
        Ok(())
    }

    async fn check_recourse_payment_for_bill(
        &self,
        bill_id: &BillId,
        identity: &IdentityWithAll,
    ) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        let bill_ids_waiting_for_recourse_payment = self
            .store
            .get_bill_ids_waiting_for_recourse_payment()
            .await?;
        let now = Timestamp::now();

        if bill_ids_waiting_for_recourse_payment
            .iter()
            .any(|id| id == bill_id)
        {
            if let Err(e) = self
                .check_bill_in_recourse_payment(bill_id, identity, now)
                .await
            {
                error!("Checking recourse payment for {bill_id} failed: {e}");
            }
        } else {
            info!(
                "Bill with id {bill_id} is not waiting for recourse payment - not checking payment"
            );
        }

        Ok(())
    }

    async fn check_bills_timeouts(&self, now: Timestamp) -> Result<()> {
        let op_codes = HashSet::from([
            BillOpCode::RequestToPay,
            BillOpCode::OfferToSell,
            BillOpCode::RequestToAccept,
            BillOpCode::RequestRecourse,
        ]);

        let bill_ids_to_check = self
            .store
            .get_bill_ids_with_op_codes_since(op_codes, Timestamp::zero())
            .await?;

        for bill_id in bill_ids_to_check {
            if let Err(e) = self.check_bill_timeouts(&bill_id, now).await {
                error!("Checking bill timeouts for {bill_id} failed: {e}");
            }
        }

        Ok(())
    }

    async fn get_past_endorsees(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<PastEndorsee>> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(current_identity_node_id)?;
        match self.store.exists(bill_id).await {
            Ok(true) => (),
            _ => {
                return Err(Error::NotFound);
            }
        };

        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;

        let bill_participants = chain
            .get_all_nodes_from_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?;
        // active identity is not part of the bill
        if !bill_participants
            .iter()
            .any(|p| p == current_identity_node_id)
        {
            debug!("caller is not a participant of bill {bill_id}");
            return Err(Error::NotFound);
        }

        let res = chain
            .get_past_endorsees_for_bill(&bill_keys, current_identity_node_id)
            .map_err(|e| Error::Protocol(e.into()))?;
        Ok(res)
    }

    async fn get_past_payments(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Vec<PastPaymentResult>> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        match self.store.exists(bill_id).await {
            Ok(true) => (),
            _ => {
                return Err(Error::NotFound);
            }
        };
        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let is_paid = self.store.is_paid(bill_id).await?;
        let bill = chain
            .get_first_version_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?;

        let res = self
            .fetch_past_payments(
                bill_id,
                caller_public_data,
                caller_keys,
                timestamp,
                &chain,
                &bill_keys,
                is_paid,
                &bill,
            )
            .await?;
        Ok(res)
    }

    async fn get_endorsements(
        &self,
        bill_id: &BillId,
        identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<Vec<Endorsement>> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        let bill = self
            .get_detail(
                bill_id,
                identity,
                caller_public_data,
                caller_keys,
                current_timestamp,
            )
            .await?;
        let result = bill.participants.endorsements;
        Ok(result)
    }

    async fn clear_bill_cache(&self) -> Result<()> {
        self.store.clear_bill_cache().await?;
        Ok(())
    }

    async fn request_to_mint(
        &self,
        bill_id: &BillId,
        mint_node_id: &NodeId,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&signer_public_data.node_id())?;
        validate_node_id_network(mint_node_id)?;
        debug!("Executing request to mint with mint {mint_node_id} for bill {bill_id}");
        let mint_cfg = &get_config().mint_config;
        // make sure the mint is a valid one - currently just checks it against the default mint
        if mint_cfg.default_mint_node_id != *mint_node_id {
            return Err(Error::Validation(ValidationError::InvalidMint(
                mint_node_id.to_string(),
            )));
        }
        // fetch data
        let identity = self.identity_store.get().await?;
        let contacts = self.contact_store.get_map().await?;
        let blockchain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let bill = self
            .get_last_version_bill(&blockchain, &bill_keys, &identity, &contacts)
            .await?;
        let is_paid = self.store.is_paid(bill_id).await?;

        let mint_anon_participant = self.get_participant_for_mint(mint_node_id, &identity).await;
        // validate using mint bill action - no point doing a mint request, if it can't be minted
        BillValidateActionData {
            blockchain: blockchain.clone(),
            drawee_node_id: bill.drawee.node_id.clone(),
            payee_node_id: bill.payee.node_id().clone(),
            endorsee_node_id: bill.endorsee.clone().map(|e| e.node_id()),
            maturity_date: bill.maturity_date.clone(),
            bill_keys: bill_keys.clone(),
            timestamp,
            signer_node_id: signer_public_data.node_id().clone(),
            is_paid,
            mode: BillValidationActionMode::Deep(BillAction::Mint(
                mint_anon_participant.clone(),
                bill.sum.clone(),
            )),
        }
        .validate()?;

        let requests_to_mint_for_bill_and_mint = self
            .mint_store
            .get_requests(&signer_public_data.node_id(), bill_id, mint_node_id)
            .await?;
        // If there are any active, or accepted (i.e. pending, accepted or offered) requests, we can't make another one
        if requests_to_mint_for_bill_and_mint.iter().any(|rtm| {
            matches!(
                rtm.status,
                MintRequestStatus::Pending
                    | MintRequestStatus::Offered
                    | MintRequestStatus::Accepted
            )
        }) {
            return Err(Error::Validation(
                ValidationError::RequestToMintForBillAndMintAlreadyActive,
            ));
        }

        // Upload existing files for the mint
        let file_urls_for_mint = self
            .upload_bill_files_for_node_id(
                bill_id,
                &bill_keys.get_private_key(),
                &mint_anon_participant.node_id().pub_key(),
                &bill_keys,
                &bill.files,
            )
            .await?;

        // Send request to mint to mint
        let bill_to_share = create_bill_to_share_with_external_party(
            bill_id,
            &blockchain,
            &bill_keys,
            &mint_anon_participant.node_id().pub_key(),
            signer_keys,
            &file_urls_for_mint,
        )
        .map_err(|e| Error::Protocol(e.into()))?;
        let mint_request_id = self
            .mint_client
            .enquire_mint_quote(&mint_cfg.default_mint_url, bill_to_share, signer_keys)
            .await?;

        // Store request to mint
        self.mint_store
            .add_request(
                &signer_public_data.node_id(),
                bill_id,
                mint_node_id,
                &mint_request_id,
                timestamp,
            )
            .await?;

        // Calculate bill and persist it to cache
        self.recalculate_and_persist_bill(
            bill_id,
            &blockchain,
            &bill_keys,
            &identity,
            signer_public_data,
            signer_keys,
            timestamp,
        )
        .await?;

        // Send notifications
        if let Err(e) = self
            .transport_service
            .send_request_to_mint_event(
                &signer_public_data.node_id(),
                &mint_anon_participant,
                &bill,
            )
            .await
        {
            error!("Couldn't send notifications for request to mint: {e}");
        }

        debug!("Executed request to mint with mint {mint_node_id} for bill {bill_id}");

        Ok(())
    }

    async fn get_mint_state(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<MintRequestState>> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(current_identity_node_id)?;
        let requests = self
            .mint_store
            .get_requests_for_bill(current_identity_node_id, bill_id)
            .await?;

        let mut req_states: Vec<MintRequestState> = requests
            .into_iter()
            .map(|req| MintRequestState {
                request: req,
                offer: None,
            })
            .collect();

        for req in req_states.iter_mut() {
            // if it's offered, or accepted, we also fetch the offer
            if matches!(
                req.request.status,
                MintRequestStatus::Offered
                    | MintRequestStatus::Accepted
                    | MintRequestStatus::MintingEnabled
            ) {
                let offer = self
                    .mint_store
                    .get_offer(&req.request.mint_request_id)
                    .await?;
                req.offer = offer;
            }
        }

        Ok(req_states)
    }

    async fn cancel_request_to_mint(
        &self,
        mint_request_id: &Uuid,
        current_identity_node_id: &NodeId,
    ) -> Result<()> {
        debug!("trying to cancel request to mint {mint_request_id}");
        validate_node_id_network(current_identity_node_id)?;
        let req = self
            .get_req_to_mint_for_node_id(mint_request_id, current_identity_node_id)
            .await?;
        // only if it's pending
        if matches!(req.status, MintRequestStatus::Pending) {
            self.mint_client
                .cancel_quote_for_mint(
                    &get_config().mint_config.default_mint_url,
                    &req.mint_request_id,
                )
                .await?;
            self.mint_store
                .update_request(
                    mint_request_id,
                    &MintRequestStatus::Cancelled {
                        timestamp: Timestamp::now(),
                    },
                )
                .await?;
            Ok(())
        } else {
            Err(Error::Validation(
                ValidationError::CancelMintRequestNotPending,
            ))
        }
    }

    async fn check_mint_state(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<()> {
        debug!("checking mint requests for bill {bill_id}");
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(current_identity_node_id)?;
        let requests = self
            .mint_store
            .get_requests_for_bill(current_identity_node_id, bill_id)
            .await?;
        for req in requests {
            if let Err(e) = self.check_mint_quote_and_update_bill_mint_state(&req).await {
                error!(
                    "Could not check mint state for {}: {e}",
                    req.mint_request_id
                );
            }
        }
        Ok(())
    }

    async fn check_mint_state_for_all_bills(&self) -> Result<()> {
        debug!("checking all not-finished mint requests");
        // get all not-finished (offered, pending, accepted) requests
        let requests = self.mint_store.get_all_active_requests().await?;
        debug!(
            "checking all not-finished mint requests ({})",
            requests.len()
        );
        for req in requests {
            if let Err(e) = self.check_mint_quote_and_update_bill_mint_state(&req).await {
                error!(
                    "Could not check mint state for {}: {e}",
                    req.mint_request_id
                );
            }
        }
        Ok(())
    }

    async fn accept_mint_offer(
        &self,
        mint_request_id: &Uuid,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<()> {
        let identity = self.identity_store.get().await?;
        debug!("trying to accept offer from request to mint {mint_request_id}");
        validate_node_id_network(&signer_public_data.node_id())?;
        let req = self
            .get_req_to_mint_for_node_id(mint_request_id, &signer_public_data.node_id())
            .await?;
        // only if it's offered
        if matches!(req.status, MintRequestStatus::Offered) {
            // and not expired
            if let Ok(Some(offer)) = self.mint_store.get_offer(&req.mint_request_id).await {
                if offer.expiration_timestamp < timestamp {
                    return Err(Error::Validation(ValidationError::AcceptMintOfferExpired));
                }
                // accept the offer
                self.mint_client
                    .resolve_quote_for_mint(
                        &get_config().mint_config.default_mint_url,
                        &req.mint_request_id,
                        ResolveMintOffer::Accept,
                    )
                    .await?;

                // endorse the bill to the mint
                let mint_anon_participant = self
                    .get_participant_for_mint(&req.mint_node_id, &identity)
                    .await;
                self.execute_bill_action(
                    &req.bill_id,
                    BillAction::Mint(mint_anon_participant, offer.discounted_sum),
                    signer_public_data,
                    signer_keys,
                    timestamp,
                )
                .await?;

                // update the state
                self.mint_store
                    .update_request(mint_request_id, &MintRequestStatus::Accepted)
                    .await?;

                // reject all other offers - but don't fail on errors
                if let Err(e) = self
                    .reject_mint_offers_except_accepted_one_for_bill(
                        &req.mint_request_id,
                        &req.bill_id,
                        &signer_public_data.node_id(),
                    )
                    .await
                {
                    error!(
                        "Error rejecting other mint offers for bill {}: {e}",
                        req.bill_id
                    );
                }
                Ok(())
            } else {
                Err(Error::NotFound)
            }
        } else {
            Err(Error::Validation(
                ValidationError::AcceptMintRequestNotOffered,
            ))
        }
    }

    async fn reject_mint_offer(
        &self,
        mint_request_id: &Uuid,
        current_identity_node_id: &NodeId,
    ) -> Result<()> {
        debug!("trying to reject offer from request to mint {mint_request_id}");
        validate_node_id_network(current_identity_node_id)?;
        let req = self
            .get_req_to_mint_for_node_id(mint_request_id, current_identity_node_id)
            .await?;
        // only if it's offered
        if matches!(req.status, MintRequestStatus::Offered) {
            self.mint_client
                .resolve_quote_for_mint(
                    &get_config().mint_config.default_mint_url,
                    &req.mint_request_id,
                    ResolveMintOffer::Reject,
                )
                .await?;
            self.mint_store
                .update_request(
                    mint_request_id,
                    &MintRequestStatus::Rejected {
                        timestamp: Timestamp::now(),
                    },
                )
                .await?;
            Ok(())
        } else {
            Err(Error::Validation(
                ValidationError::RejectMintRequestNotOffered,
            ))
        }
    }

    async fn dev_mode_get_full_bill_chain(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<BillBlockPlaintextWrapper>> {
        // if dev mode is off - we return an error
        if !get_config().dev_mode_config.on {
            error!("Called dev mode operation with dev mode disabled - please enable!");
            return Err(Error::Validation(ValidationError::InvalidOperation));
        }

        validate_bill_id_network(bill_id)?;
        validate_node_id_network(current_identity_node_id)?;

        // if there is no such bill, we return an error
        match self.store.exists(bill_id).await {
            Ok(true) => (),
            _ => {
                return Err(Error::NotFound);
            }
        };

        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let bill_participant_node_ids = chain
            .get_all_nodes_from_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?;

        // if currently active identity is not part of the bill, we can't access it
        if !bill_participant_node_ids
            .iter()
            .any(|p| p == current_identity_node_id)
        {
            return Err(Error::NotFound);
        }

        let plaintext_chain = chain
            .get_chain_with_plaintext_block_data(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?;

        Ok(plaintext_chain)
    }

    async fn share_bill_with_court(
        &self,
        bill_id: &BillId,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        court_node_id: &NodeId,
    ) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&signer_public_data.node_id())?;
        let court_url = get_config().court_config.default_url.clone();
        debug!(
            "Executing share with court {court_node_id} with court {} for bill {bill_id}",
            court_url
        );
        let blockchain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;
        let participants = blockchain
            .get_all_nodes_from_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?;
        // Can only share bills we are part of
        if !participants.contains(&signer_public_data.node_id()) {
            return Err(Error::NotFound);
        }

        let identity = self.identity_store.get().await?;
        let contacts = self.contact_store.get_map().await?;
        let bill = self
            .get_last_version_bill(&blockchain, &bill_keys, &identity, &contacts)
            .await?;

        // Upload existing files for the court to our relay
        let file_urls_for_court = self
            .upload_bill_files_for_node_id(
                bill_id,
                &bill_keys.get_private_key(),
                &court_node_id.pub_key(),
                &bill_keys,
                &bill.files,
            )
            .await?;

        // Send request to share with court
        let bill_to_share = create_bill_to_share_with_external_party(
            bill_id,
            &blockchain,
            &bill_keys,
            &court_node_id.pub_key(),
            signer_keys,
            &file_urls_for_court,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.court_client
            .share_with_court(&court_url, bill_to_share, signer_keys)
            .await?;

        debug!(
            "Executed share with court {court_node_id} with court {} for bill {bill_id}",
            court_url
        );
        Ok(())
    }

    async fn get_bill_history(
        &self,
        bill_id: &BillId,
        local_identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<BillHistory> {
        let detail = self
            .get_detail(
                bill_id,
                local_identity,
                caller_public_data,
                caller_keys,
                current_timestamp,
            )
            .await?;
        Ok(detail.history)
    }

    async fn get_local_bill_chain(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
    ) -> Result<BillBlockchain> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let bill_keys = self.store.get_keys(bill_id).await?;

        // if caller is not part of the bill, they can't access it
        if !chain
            .get_all_nodes_from_bill(&bill_keys)
            .map_err(|e| Error::Protocol(e.into()))?
            .iter()
            .any(|p| p == &caller_public_data.node_id())
        {
            debug!("caller is not a participant of bill {bill_id}");
            return Err(Error::NotFound);
        }

        Ok(chain)
    }

    fn mempool_link(&self, address: &BitcoinAddress) -> String {
        self.bitcoin_client.get_mempool_link_for_address(address)
    }

    fn link_to_pay(&self, address: &BitcoinAddress, sum: &Sum, bill_id: &BillId) -> String {
        self.bitcoin_client.generate_link_to_pay(
            address,
            sum,
            &format!("Payment in relation to a bill {}", bill_id),
        )
    }

    async fn check_and_estimate_btc_sweep(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        source_address: &BitcoinAddress,
        destination_address: &BitcoinAddress,
    ) -> Result<SweepEstimate> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        // fetch past payments, to check if this is a valid payment
        let past_payments = self
            .get_past_payments(bill_id, caller_public_data, caller_keys, Timestamp::now())
            .await?;
        let mut descriptor = None;
        for pp in past_payments.iter() {
            match pp {
                PastPaymentResult::Sell(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
                PastPaymentResult::Payment(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
                PastPaymentResult::Recourse(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
            };
        }
        let Some(desc) = descriptor else {
            return Err(Error::Validation(ValidationError::NoPaymentForSweep));
        };
        let estimate = self
            .bitcoin_client
            .check_and_estimate_sweep(&desc, destination_address)
            .await?;
        Ok(estimate)
    }

    async fn btc_sweep(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        source_address: &BitcoinAddress,
        destination_address: &BitcoinAddress,
        fee: u64,
    ) -> Result<SweepResult> {
        validate_bill_id_network(bill_id)?;
        validate_node_id_network(&caller_public_data.node_id())?;
        // fetch past payments, to check if this is a valid payment
        let past_payments = self
            .get_past_payments(bill_id, caller_public_data, caller_keys, Timestamp::now())
            .await?;
        let mut descriptor = None;
        for pp in past_payments.iter() {
            match pp {
                PastPaymentResult::Sell(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
                PastPaymentResult::Payment(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
                PastPaymentResult::Recourse(data) => {
                    if &data.address_to_pay == source_address {
                        descriptor = data.private_descriptor_to_spend.to_owned()
                    }
                }
            };
        }
        let Some(desc) = descriptor else {
            return Err(Error::Validation(ValidationError::NoPaymentForSweep));
        };
        let res = self
            .bitcoin_client
            .sweep_funds(&desc, destination_address, fee)
            .await?;
        Ok(res)
    }

    async fn get_address_derivation_metadata_for_payment_request(
        &self,
        bill_id: &BillId,
    ) -> Result<AddressDerivationMetadataForPaymentRequest> {
        validate_bill_id_network(bill_id)?;
        let chain = self.blockchain_store.get_chain(bill_id).await?;
        let latest_block = chain.get_latest_block();
        Ok(AddressDerivationMetadataForPaymentRequest {
            block_id: BlockId::next_from_previous_block_id(&latest_block.id()),
            previous_hash: latest_block.hash().to_owned(),
        })
    }
}
