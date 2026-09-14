use super::Result;
use crate::constants::{COMPANY_LOGO_FILE_FIELD, COMPANY_PROOF_OF_REGISTRATION_FILE_FIELD};
use crate::external::email::EmailClientApi;
use crate::external::file_storage::FileStorageClientApi;
use crate::get_config;
use crate::service::Error;
use crate::service::file_reference_helper::{company_file_context, encrypt_upload_and_track_file};
use crate::service::file_server_service::{
    configured_blossom_servers, download_file_with_fallback,
};
use crate::service::file_upload_service::UploadFileType;
use crate::service::transport_service::{BcrMetadata, NostrContactData, TransportServiceApi};
use crate::util::{self, validate_node_id_network};
use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::application::company::{
    Company, CompanySignatory, CompanySignatoryStatus, CompanyStatus, LocalSignatoryOverrideStatus,
};
use bcr_ebill_core::application::contact::Contact;
use bcr_ebill_core::application::identity::ActiveIdentityState;
use bcr_ebill_core::application::{ServiceTraitBounds, ValidationError};
use bcr_ebill_core::protocol::EditOptionalFieldMode;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::blockchain::bill::ContactType;
use bcr_ebill_core::protocol::blockchain::company::{
    CompanyBlock, CompanyBlockchain,
    block::{
        CompanyCreateBlockData, CompanyIdentityProofBlockData, CompanyInviteSignatoryBlockData,
        CompanyRemoveSignatoryBlockData, CompanySignatoryAcceptInviteBlockData,
        CompanySignatoryRejectInviteBlockData, CompanyUpdateBlockData, SignatoryType,
    },
    chain::CompanyBlockPlaintextWrapper,
};
use bcr_ebill_core::protocol::blockchain::company::{CompanyOpCode, CompanyValidateActionData};
use bcr_ebill_core::protocol::blockchain::identity::{
    IdentityAcceptSignatoryInviteBlockData, IdentityBlock, IdentityCreateCompanyBlockData,
    IdentityInviteSignatoryBlockData, IdentityOpCode, IdentityRejectSignatoryInviteBlockData,
    IdentityRemoveSignatoryBlockData, IdentityValidateActionData,
};
use bcr_ebill_core::protocol::blockchain::identity::{IdentityBlockchain, IdentityType};
use bcr_ebill_core::protocol::blockchain::{Block, Blockchain};
use bcr_ebill_core::protocol::crypto::{self, BcrKeys, DeriveKeypair};
use bcr_ebill_core::protocol::{Address, Name, Zip};
use bcr_ebill_core::protocol::{BlockId, Email};
use bcr_ebill_core::protocol::{City, ProtocolValidationError};
use bcr_ebill_core::protocol::{Country, blockchain};
use bcr_ebill_core::protocol::{Date, EmailIdentityProofData, SignedIdentityProof};
use bcr_ebill_core::protocol::{File, PostalAddress};
use bcr_ebill_core::protocol::{Identification, Validate};
use bcr_ebill_core::protocol::{
    PublicKey, SecretKey,
    event::{CompanyChainEvent, IdentityChainEvent},
};
use bcr_ebill_persistence::ContactStoreApi;
use bcr_ebill_persistence::FileReferenceStoreApi;
use bcr_ebill_persistence::company::{CompanyChainStoreApi, CompanyStoreApi};
use bcr_ebill_persistence::file_upload::FileUploadStoreApi;
use bcr_ebill_persistence::identity::{IdentityChainStoreApi, IdentityStoreApi};
use bcr_ebill_persistence::nostr::NostrContactStoreApi;
use bcr_ebill_persistence::notification::EmailNotificationStoreApi;
use bitcoin::base58;
use log::{debug, error, info};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio_with_wasm::alias as tokio;
use uuid::Uuid;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait CompanyServiceApi: ServiceTraitBounds {
    /// List signatories for company
    async fn list_signatories(&self, id: &NodeId) -> Result<Vec<(CompanySignatory, Contact)>>;

    /// Search companies
    async fn search(&self, search_term: &str) -> Result<Vec<Company>>;

    /// Get a list of companies
    async fn get_list_of_companies(&self) -> Result<Vec<Company>>;

    /// Get a company by id
    async fn get_company_by_id(&self, id: &NodeId) -> Result<Company>;

    /// Get a company and it's keys by id
    async fn get_company_and_keys_by_id(&self, id: &NodeId) -> Result<(Company, BcrKeys)>;

    /// Create a new company key pair
    async fn create_company_keys(&self) -> Result<NodeId>;

    /// Create a new company, `create_company_keys` needs to be called before to get a key pair
    async fn create_company(
        &self,
        id: NodeId,
        name: Name,
        country_of_registration: Option<Country>,
        city_of_registration: Option<City>,
        postal_address: PostalAddress,
        email: Email,
        registration_number: Option<Identification>,
        registration_date: Option<Date>,
        proof_of_registration_file_upload_id: Option<Uuid>,
        logo_file_upload_id: Option<Uuid>,
        creator_email: Email,
        timestamp: Timestamp,
    ) -> Result<Company>;

    /// Changes the given company fields for the given company, if they are set
    async fn edit_company(
        &self,
        id: &NodeId,
        name: Option<Name>,
        email: Option<Email>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        country_of_registration: EditOptionalFieldMode<Country>,
        city_of_registration: EditOptionalFieldMode<City>,
        registration_number: EditOptionalFieldMode<Identification>,
        registration_date: EditOptionalFieldMode<Date>,
        logo_file_upload_id: EditOptionalFieldMode<Uuid>,
        proof_of_registration_file_upload_id: EditOptionalFieldMode<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// Invite another signatory to the given company
    async fn invite_signatory(
        &self,
        id: &NodeId,
        signatory_node_id: NodeId,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// Removes a signatory from the given company
    async fn remove_signatory(
        &self,
        id: &NodeId,
        signatory_node_id: NodeId,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// opens and decrypts the attached file from the given company
    async fn open_and_decrypt_file(
        &self,
        company: Company,
        id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>>;

    /// Shares derived keys for given company contact information.
    async fn share_contact_details(&self, share_to: &NodeId, company_id: NodeId) -> Result<()>;

    /// If dev mode is on, return the full company chain with decrypted data
    async fn dev_mode_get_full_company_chain(
        &self,
        id: &NodeId,
    ) -> Result<Vec<CompanyBlockPlaintextWrapper>>;

    /// Publishes this company's contact to the nostr profile
    async fn publish_contact(&self, company: &Company, keys: &BcrKeys) -> Result<()>;

    /// Change the email of the signatory (local identity only)
    async fn change_signatory_email(&self, id: &NodeId, email: &Email) -> Result<()>;

    /// Confirm a new email address for a company id
    async fn confirm_email(&self, id: &NodeId, email: &Email) -> Result<()>;

    /// Verify confirmation of an email address with the sent confirmation code for a company id
    async fn verify_email(&self, id: &NodeId, confirmation_code: &str) -> Result<()>;

    /// Get email confirmations for the identity and company
    async fn get_email_confirmations(
        &self,
        id: &NodeId,
    ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>>;

    /// Get active company invites for the current identity
    async fn get_active_company_invites(&self) -> Result<Vec<Company>>;

    /// Accept an invite to a company (needs a confirmed email)
    async fn accept_company_invite(
        &self,
        id: &NodeId,
        email: &Email,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// Reject an invite to a company
    async fn reject_company_invite(&self, id: &NodeId, timestamp: Timestamp) -> Result<()>;

    /// Locally hide a removed signatory
    async fn locally_hide_signatory(&self, id: &NodeId, signatory_node_id: &NodeId) -> Result<()>;

    /// Filter out locally hidden signatories for display
    async fn filter_out_locally_hidden_signatories(
        &self,
        company_id: &NodeId,
        signatories: &mut Vec<CompanySignatory>,
    ) -> Result<()>;
}

/// The company service is responsible for managing the companies
#[derive(Clone)]
pub struct CompanyService {
    store: Arc<dyn CompanyStoreApi>,
    file_upload_store: Arc<dyn FileUploadStoreApi>,
    file_upload_client: Arc<dyn FileStorageClientApi>,
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
    identity_store: Arc<dyn IdentityStoreApi>,
    contact_store: Arc<dyn ContactStoreApi>,
    nostr_contact_store: Arc<dyn NostrContactStoreApi>,
    identity_blockchain_store: Arc<dyn IdentityChainStoreApi>,
    company_blockchain_store: Arc<dyn CompanyChainStoreApi>,
    transport_service: Arc<dyn TransportServiceApi>,
    email_client: Arc<dyn EmailClientApi>,
    email_notification_store: Arc<dyn EmailNotificationStoreApi>,
}

impl CompanyService {
    pub fn new(
        store: Arc<dyn CompanyStoreApi>,
        file_upload_store: Arc<dyn FileUploadStoreApi>,
        file_upload_client: Arc<dyn FileStorageClientApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        identity_store: Arc<dyn IdentityStoreApi>,
        contact_store: Arc<dyn ContactStoreApi>,
        nostr_contact_store: Arc<dyn NostrContactStoreApi>,
        identity_blockchain_store: Arc<dyn IdentityChainStoreApi>,
        company_blockchain_store: Arc<dyn CompanyChainStoreApi>,
        transport_service: Arc<dyn TransportServiceApi>,
        email_client: Arc<dyn EmailClientApi>,
        email_notification_store: Arc<dyn EmailNotificationStoreApi>,
    ) -> Self {
        Self {
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_blockchain_store,
            company_blockchain_store,
            transport_service,
            email_client,
            email_notification_store,
        }
    }

    async fn process_upload_file(
        &self,
        upload_id: &Option<Uuid>,
        id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        upload_file_type: UploadFileType,
        field_name: &str,
    ) -> Result<Option<File>> {
        if let Some(upload_id) = upload_id {
            debug!("processing upload file for company {id}: {upload_id:?}");
            let (file_name, file_bytes) = &self
                .file_upload_store
                .read_temp_upload_file(upload_id)
                .await
                .map_err(|_| {
                    crate::service::Error::Validation(ValidationError::NoFileForFileUploadId)
                })?;
            // validate file size for upload file type
            if !upload_file_type.check_file_size(file_bytes.len()) {
                return Err(crate::service::Error::Validation(
                    ProtocolValidationError::FileIsTooBig(upload_file_type.max_file_size()).into(),
                ));
            }
            let file = self
                .encrypt_and_upload_file(file_name, file_bytes, id, public_key, signer, field_name)
                .await?;
            return Ok(Some(file));
        }
        Ok(None)
    }

    async fn encrypt_and_upload_file(
        &self,
        file_name: &Name,
        file_bytes: &[u8],
        id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        field_name: &str,
    ) -> Result<File> {
        encrypt_upload_and_track_file(
            &self.file_reference_store,
            &self.file_upload_client,
            &self.transport_service,
            &configured_blossom_servers(&get_config().nostr_config),
            file_name,
            file_bytes,
            public_key,
            signer,
            id,
            company_file_context(id, field_name),
            None,
            "company",
        )
        .await
    }

    async fn populate_block(
        &self,
        company: &Company,
        chain: &CompanyBlockchain,
        keys: &BcrKeys,
        new_signatory: Option<NodeId>,
    ) -> Result<()> {
        self.transport_service
            .block_transport()
            .send_company_chain_events(CompanyChainEvent::new(
                &company.id,
                chain,
                keys,
                new_signatory,
                true,
            ))
            .await?;
        Ok(())
    }

    async fn on_company_contact_change(&self, company: &Company, keys: &BcrKeys) -> Result<()> {
        debug!("Company change");
        self.publish_contact(company, keys).await
    }

    async fn create_identity_proof_block(
        &self,
        proof: SignedIdentityProof,
        data: EmailIdentityProofData,
        company: &Company,
        identity_keys: &BcrKeys,
        company_keys: &BcrKeys,
        chain: &mut CompanyBlockchain,
        reference_block: &Option<BlockId>,
    ) -> Result<()> {
        let new_block = CompanyBlock::create_block_for_identity_proof(
            company.id.clone(),
            chain.get_latest_block(),
            &CompanyIdentityProofBlockData {
                proof,
                data,
                reference_block: reference_block.to_owned(),
            },
            identity_keys,
            company_keys,
            Timestamp::now(),
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.validate_and_add_block(&company.id, chain, new_block.clone())
            .await?;
        self.populate_block(company, chain, company_keys, None)
            .await?;

        Ok(())
    }

    /// If mandatory email confirmations are not disabled, check that there is a confirmed email and the
    /// given email is a confirmed one for the company signatory
    async fn check_confirmed_email(
        &self,
        node_id: &NodeId,
        company_id: &NodeId,
        email: &Email,
    ) -> Result<(SignedIdentityProof, EmailIdentityProofData)> {
        if get_config().dev_mode_config.mandatory_email_confirmations {
            // Make sure there is a confirmed email
            let email_confirmations = self.store.get_email_confirmations(company_id).await?;
            if email_confirmations.is_empty() {
                return Err(Error::Validation(
                    ValidationError::NoConfirmedEmailForIdentIdentity,
                ));
            }

            // Given email has to be a confirmed email
            for ec in email_confirmations.iter() {
                if &ec.1.email == email {
                    return Ok((ec.0.to_owned(), ec.1.to_owned()));
                }
            }

            // No email found - fail
            Err(Error::Validation(
                ValidationError::NoConfirmedEmailForIdentIdentity,
            ))
        } else {
            // if mandatory email confirmations are disabled, create self-signed email confirmation
            let identity_keys = self.identity_store.get_key_pair().await?;

            let self_signed_identity = EmailIdentityProofData {
                node_id: node_id.to_owned(),
                company_node_id: Some(company_id.to_owned()),
                email: email.to_owned(),
                created_at: Timestamp::now(),
            };
            let proof = self_signed_identity.sign(node_id, &identity_keys.get_private_key())?;
            self.store
                .set_email_confirmation(company_id, &proof, &self_signed_identity)
                .await?;

            Ok((proof, self_signed_identity))
        }
    }

    async fn validate_and_add_identity_block(
        &self,
        chain: &mut IdentityBlockchain,
        new_block: IdentityBlock,
    ) -> Result<()> {
        let try_add_block = chain.try_add_block(new_block.clone());
        if try_add_block && chain.is_chain_valid() {
            self.identity_blockchain_store.add_block(&new_block).await?;
            Ok(())
        } else {
            Err(Error::Protocol(blockchain::Error::BlockchainInvalid.into()))
        }
    }

    async fn validate_and_add_block(
        &self,
        company_id: &NodeId,
        chain: &mut CompanyBlockchain,
        new_block: CompanyBlock,
    ) -> Result<()> {
        let try_add_block = chain.try_add_block(new_block.clone());
        if try_add_block && chain.is_chain_valid() {
            self.company_blockchain_store
                .add_block(company_id, &new_block)
                .await?;
            Ok(())
        } else {
            Err(Error::Protocol(blockchain::Error::BlockchainInvalid.into()))
        }
    }
}

/// Derives a company contact encryption key, encrypts the contact data with it and returns the BCR metadata.
fn get_bcr_data(
    company: &Company,
    keys: &BcrKeys,
    relays: Vec<url::Url>,
    mint_url: Option<url::Url>,
) -> Result<BcrMetadata> {
    let derived_keys = keys.derive_company_keypair()?;
    let contact = Contact {
        t: ContactType::Company,
        node_id: company.id.clone(),
        name: company.name.clone(),
        email: Some(company.email.to_owned()),
        postal_address: Some(company.postal_address.clone()),
        date_of_birth_or_registration: company.registration_date.to_owned(),
        country_of_birth_or_registration: company.country_of_registration.to_owned(),
        city_of_birth_or_registration: company.city_of_registration.to_owned(),
        identification_number: company.registration_number.to_owned(),
        avatar_file: company.logo_file.to_owned(),
        proof_document_file: None,
        nostr_relays: relays,
        is_logical: false,
        mint_url,
    };
    let payload = serde_json::to_string(&contact)?;
    let encrypted = base58::encode(&crypto::encrypt_ecies(
        payload.as_bytes(),
        &derived_keys.public_key(),
    )?);
    Ok(BcrMetadata {
        contact_data: encrypted,
    })
}

impl ServiceTraitBounds for CompanyService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompanyServiceApi for CompanyService {
    async fn list_signatories(&self, id: &NodeId) -> Result<Vec<(CompanySignatory, Contact)>> {
        validate_node_id_network(id)?;
        if !self.store.exists(id).await {
            return Err(crate::service::Error::NotFound);
        }
        let identity = self.identity_store.get().await?;
        // don't return signatories for anon
        if identity.t == IdentityType::Anon {
            return Err(crate::service::Error::NotFound);
        }
        let company = self.store.get(id).await?;
        let contacts = self.contact_store.get_map().await?;

        // we add all where we have a contact
        let mut signatory_contacts: Vec<(CompanySignatory, Contact)> = company
            .signatories
            .iter()
            .filter_map(|signatory| {
                contacts
                    .get(&signatory.node_id)
                    .map(|contact| (signatory.clone(), contact.clone()))
            })
            .collect();

        // if we are signatory and not yet in signatory contacts, add our identity contact
        if let Some(self_signatory) = company
            .signatories
            .iter()
            .find(|s| s.node_id == identity.node_id)
            && !signatory_contacts
                .iter()
                .any(|c| c.0.node_id == identity.node_id)
        {
            // we force person for this as it will be thrown out in later validation
            signatory_contacts.push((
                self_signatory.clone(),
                identity.as_contact(Some(ContactType::Person)),
            ));
        }

        // if we are still missing some signatory details try to fill them from nostr contacts
        if signatory_contacts.len() < company.signatories.len() {
            let missing = company
                .signatories
                .iter()
                .filter(|s| !signatory_contacts.iter().any(|c| c.0.node_id == s.node_id))
                .map(|s| (s.node_id.to_owned(), s.to_owned()))
                .collect::<HashMap<NodeId, CompanySignatory>>();

            let nostr_contacts: Vec<Contact> = self
                .nostr_contact_store
                .by_node_ids(missing.keys().cloned().collect())
                .await?
                .into_iter()
                .filter_map(|c| c.into_contact(Some(ContactType::Person)))
                .collect();

            for nostr_contact in nostr_contacts.iter() {
                if let Some(signatory) = missing.get(&nostr_contact.node_id) {
                    signatory_contacts.push((signatory.to_owned(), nostr_contact.to_owned()));
                }
            }
        }

        Ok(signatory_contacts)
    }

    async fn search(&self, search_term: &str) -> Result<Vec<Company>> {
        let results = self.store.search(search_term).await?;
        Ok(results)
    }

    async fn get_list_of_companies(&self) -> Result<Vec<Company>> {
        let results = self.store.get_all().await?;
        let companies: Vec<Company> = results
            .into_iter()
            .map(|(_id, (company, _keys))| company)
            .collect();
        Ok(companies)
    }

    async fn get_company_and_keys_by_id(&self, id: &NodeId) -> Result<(Company, BcrKeys)> {
        validate_node_id_network(id)?;
        if !self.store.exists(id).await {
            return Err(crate::service::Error::NotFound);
        }

        // don't return company for anon user
        let full_identity = self.identity_store.get_full().await?;
        if full_identity.identity.t == IdentityType::Anon {
            return Err(crate::service::Error::NotFound);
        }
        let company = self.store.get(id).await?;
        let keys = self.store.get_key_pair(id).await?;
        Ok((company, keys))
    }

    async fn get_company_by_id(&self, id: &NodeId) -> Result<Company> {
        validate_node_id_network(id)?;
        let (company, _keys) = self.get_company_and_keys_by_id(id).await?;
        Ok(company)
    }

    async fn create_company_keys(&self) -> Result<NodeId> {
        let company_keys = BcrKeys::new();
        let id = NodeId::new(company_keys.pub_key(), get_config().bitcoin_network());
        self.store.save_key_pair(&id, &company_keys).await?;
        Ok(id)
    }

    async fn create_company(
        &self,
        id: NodeId,
        name: Name,
        country_of_registration: Option<Country>,
        city_of_registration: Option<City>,
        postal_address: PostalAddress,
        email: Email,
        registration_number: Option<Identification>,
        registration_date: Option<Date>,
        proof_of_registration_file_upload_id: Option<Uuid>,
        logo_file_upload_id: Option<Uuid>,
        creator_email: Email,
        timestamp: Timestamp,
    ) -> Result<Company> {
        debug!("creating company by creator {}", creator_email);
        let company_keys = self.store.get_key_pair(&id).await?;

        // Register the company's identity with the transport layer before any file uploads,
        // so that file metadata can be published with the correct signer.
        self.transport_service
            .add_identity(&id, &company_keys)
            .await?;

        let full_identity = self.identity_store.get_full().await?;
        // company can only be created by identified identity
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }

        // check if email is confirmed
        let (proof, data) = self
            .check_confirmed_email(&full_identity.identity.node_id, &id, &creator_email)
            .await?;

        let proof_of_registration_file = self
            .process_upload_file(
                &proof_of_registration_file_upload_id,
                &id,
                &company_keys.pub_key(),
                &company_keys,
                UploadFileType::Document,
                COMPANY_PROOF_OF_REGISTRATION_FILE_FIELD,
            )
            .await?;

        let logo_file = self
            .process_upload_file(
                &logo_file_upload_id,
                &id,
                &company_keys.pub_key(),
                &company_keys,
                UploadFileType::Picture,
                COMPANY_LOGO_FILE_FIELD,
            )
            .await?;

        let status = CompanyStatus::Active; // we're creator, so it's an active company
        let company = Company {
            id: id.clone(),
            name: name.clone(),
            country_of_registration: country_of_registration.clone(),
            city_of_registration: city_of_registration.clone(),
            postal_address: postal_address.clone(),
            email: email.clone(),
            registration_number: registration_number.clone(),
            registration_date: registration_date.clone(),
            proof_of_registration_file: proof_of_registration_file.clone(),
            logo_file: logo_file.clone(),
            signatories: vec![CompanySignatory {
                node_id: full_identity.identity.node_id.clone(),
                t: SignatoryType::Solo,
                status: CompanySignatoryStatus::InviteAcceptedIdentityProven {
                    ts: timestamp,
                    proof: proof.clone(),
                    data: data.clone(),
                },
            }], // add caller as signatory
            creation_time: timestamp,
            status,
        };
        self.store.insert(&company).await?;

        let mut company_chain = CompanyBlockchain::new(
            &CompanyCreateBlockData {
                id: id.clone(),
                name,
                country_of_registration,
                city_of_registration,
                postal_address,
                email,
                registration_number,
                registration_date,
                proof_of_registration_file,
                logo_file,
                creation_time: timestamp,
                creator: full_identity.identity.node_id.clone(),
            },
            &full_identity.key_pair,
            &company_keys,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;
        let create_company_block = company_chain.get_first_block();

        let mut identity_chain = self.identity_blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: full_identity.identity.node_id.clone(),
            op: IdentityOpCode::CreateCompany,
            keys: full_identity.key_pair.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_block = identity_chain.get_latest_block();
        let new_block = IdentityBlock::create_block_for_create_company(
            previous_block,
            &IdentityCreateCompanyBlockData {
                company_id: id.clone(),
                company_key: company_keys.get_private_key(),
                block_hash: create_company_block.hash.clone(),
            },
            &full_identity.key_pair,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.company_blockchain_store
            .add_block(&id, create_company_block)
            .await?;

        self.populate_block(&company, &company_chain, &company_keys, None)
            .await?;

        self.validate_and_add_identity_block(&mut identity_chain, new_block.clone())
            .await?;

        self.transport_service
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(
                &full_identity.identity.node_id,
                &new_block,
                &full_identity.key_pair,
            ))
            .await?;

        // publish our company contact to nostr
        self.on_company_contact_change(&company, &company_keys)
            .await?;

        // add to contacts without files if it's not there already, but don't fail
        if let Ok(None) = self.contact_store.get(&id).await
            && let Err(e) = self
                .contact_store
                .insert(
                    &id,
                    Contact {
                        t: ContactType::Company,
                        node_id: id.to_owned(),
                        name: company.name.clone(),
                        email: Some(company.email.clone()),
                        postal_address: Some(company.postal_address.clone()),
                        date_of_birth_or_registration: company.registration_date.clone(),
                        country_of_birth_or_registration: company.country_of_registration.clone(),
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
            error!("Could not create contact for added company {id}: {e}");
        }

        let reference_block = create_company_block.id();
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: full_identity.identity.node_id.clone(),
            op: CompanyOpCode::IdentityProof,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: Some((proof.clone(), data.clone(), Some(reference_block))),
        }
        .validate()?;
        // Create and populate company proof block
        self.create_identity_proof_block(
            proof,
            data,
            &company,
            &full_identity.key_pair,
            &company_keys,
            &mut company_chain,
            &Some(reference_block),
        )
        .await?;

        debug!("company with id {id} created");

        // clean up temporary file uploads, if there are any, logging any errors
        for upload_id in [proof_of_registration_file_upload_id, logo_file_upload_id]
            .iter()
            .flatten()
        {
            if let Err(e) = self
                .file_upload_store
                .remove_temp_upload_folder(upload_id)
                .await
            {
                error!("Error while cleaning up temporary file uploads for {upload_id}: {e}");
            }
        }

        Ok(company)
    }

    async fn edit_company(
        &self,
        id: &NodeId,
        name: Option<Name>,
        email: Option<Email>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        country_of_registration: EditOptionalFieldMode<Country>,
        city_of_registration: EditOptionalFieldMode<City>,
        registration_number: EditOptionalFieldMode<Identification>,
        registration_date: EditOptionalFieldMode<Date>,
        logo_file_upload_id: EditOptionalFieldMode<Uuid>,
        proof_of_registration_file_upload_id: EditOptionalFieldMode<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!("editing company with id: {id}");
        validate_node_id_network(id)?;
        if !self.store.exists(id).await {
            debug!("company with id {id} does not exist");
            return Err(super::Error::NotFound);
        }
        let full_identity = self.identity_store.get_full().await?;
        // company can only be edited by identified identity
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }
        let node_id = full_identity.identity.node_id;
        let mut company = self.store.get(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;

        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: node_id.clone(),
            op: CompanyOpCode::Update,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: None,
        }
        .validate()?;

        let mut changed = false;

        if let Some(ref name_to_set) = name {
            company.name = name_to_set.clone();
            changed = true;
        }

        if let Some(ref email_to_set) = email {
            company.email = email_to_set.clone();
            changed = true;
        }

        if let Some(ref postal_address_city_to_set) = city {
            company.postal_address.city = postal_address_city_to_set.clone();
            changed = true;
        }

        if let Some(ref postal_address_country_to_set) = country {
            company.postal_address.country = postal_address_country_to_set.clone();
            changed = true;
        }

        if let Some(ref postal_address_address_to_set) = address {
            company.postal_address.address = postal_address_address_to_set.clone();
            changed = true;
        }

        util::handle_optional_field(&mut company.postal_address.zip, &zip, &mut changed);

        util::handle_optional_field(
            &mut company.country_of_registration,
            &country_of_registration,
            &mut changed,
        );

        util::handle_optional_field(
            &mut company.city_of_registration,
            &city_of_registration,
            &mut changed,
        );

        util::handle_optional_field(
            &mut company.registration_date,
            &registration_date,
            &mut changed,
        );

        util::handle_optional_field(
            &mut company.registration_number,
            &registration_number,
            &mut changed,
        );

        let logo_file = match logo_file_upload_id {
            EditOptionalFieldMode::Set(logo_file_upload_id) => {
                let logo_file = self
                    .process_upload_file(
                        &Some(logo_file_upload_id),
                        id,
                        &company_keys.pub_key(),
                        &company_keys,
                        UploadFileType::Picture,
                        COMPANY_LOGO_FILE_FIELD,
                    )
                    .await?;

                if logo_file.is_some() {
                    company.logo_file = logo_file.clone();
                    changed = true;
                }
                logo_file
            }
            EditOptionalFieldMode::Unset => {
                // remove the logo
                company.logo_file = None;
                changed = true;
                None
            }
            EditOptionalFieldMode::Ignore => {
                // nothing to do
                None
            }
        };

        let proof_of_registration_file = match proof_of_registration_file_upload_id {
            EditOptionalFieldMode::Set(proof_of_registration_file_upload_id) => {
                let proof_of_registration_file = self
                    .process_upload_file(
                        &Some(proof_of_registration_file_upload_id),
                        id,
                        &company_keys.pub_key(),
                        &company_keys,
                        UploadFileType::Document,
                        COMPANY_PROOF_OF_REGISTRATION_FILE_FIELD,
                    )
                    .await?;

                if proof_of_registration_file.is_some() {
                    company.proof_of_registration_file = proof_of_registration_file.clone();
                    changed = true;
                }
                proof_of_registration_file
            }
            EditOptionalFieldMode::Unset => {
                // remove the proof of registration
                company.proof_of_registration_file = None;
                changed = true;
                None
            }
            EditOptionalFieldMode::Ignore => {
                // nothing to do
                None
            }
        };

        if !changed {
            log::warn!("Called Company Change without any changes - returning");
            return Ok(());
        }

        self.store.update(id, &company).await?;

        let previous_block = company_chain.get_latest_block();
        let new_block = CompanyBlock::create_block_for_update(
            id.to_owned(),
            previous_block,
            &CompanyUpdateBlockData {
                name,
                email,
                country,
                city,
                zip,
                address,
                country_of_registration,
                city_of_registration,
                registration_number,
                registration_date,
                logo_file: logo_file_upload_id.swap_value(logo_file),
                proof_of_registration_file: proof_of_registration_file_upload_id
                    .swap_value(proof_of_registration_file),
            },
            &full_identity.key_pair,
            &company_keys,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;
        self.validate_and_add_block(id, &mut company_chain, new_block.clone())
            .await?;
        self.populate_block(&company, &company_chain, &company_keys, None)
            .await?;

        // publish our company contact to nostr
        self.on_company_contact_change(&company, &company_keys)
            .await?;

        debug!("company with id {id} updated");

        // clean up temporary file uploads, if there are any, logging any errors
        for upload_id in [proof_of_registration_file_upload_id, logo_file_upload_id]
            .iter()
            .filter_map(|mode| {
                if let EditOptionalFieldMode::Set(v) = mode {
                    Some(v)
                } else {
                    None
                }
            })
        {
            if let Err(e) = self
                .file_upload_store
                .remove_temp_upload_folder(upload_id)
                .await
            {
                error!("Error while cleaning up temporary file uploads for {upload_id}: {e}");
            }
        }

        Ok(())
    }

    async fn invite_signatory(
        &self,
        id: &NodeId,
        signatory_node_id: NodeId,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!(
            "adding signatory {} to company with id: {id}",
            signatory_node_id
        );
        validate_node_id_network(id)?;
        validate_node_id_network(&signatory_node_id)?;
        if !self.store.exists(id).await {
            return Err(super::Error::NotFound);
        }
        let full_identity = self.identity_store.get_full().await?;
        // only non-anon identities can add signatories
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }

        let mut company = self.store.get(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;
        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: full_identity.identity.node_id.clone(),
            op: CompanyOpCode::InviteSignatory,
            company_keys: company_keys.clone(),
            invitee: Some(signatory_node_id.clone()),
            removee: None,
            identity_proof_data: None,
        }
        .validate()?;

        let contacts = self.contact_store.get_map().await?;
        let is_in_contacts = contacts.iter().any(|(node_id, contact)| {
            *node_id == signatory_node_id && contact.t == ContactType::Person // only non-anon persons can be added
        });
        if !is_in_contacts {
            return Err(super::Error::Validation(
                ValidationError::SignatoryNotInContacts(signatory_node_id.to_string()),
            ));
        }

        // If the signatory already exists - set to invited
        if let Some(signatory) = company
            .signatories
            .iter_mut()
            .find(|s| s.node_id == signatory_node_id)
        {
            signatory.status = CompanySignatoryStatus::Invited {
                ts: timestamp,
                inviter: full_identity.identity.node_id.clone(),
            };
        } else {
            // Otherwise, add as invited
            company.signatories.push(CompanySignatory {
                node_id: signatory_node_id.clone(),
                t: SignatoryType::Solo,
                status: CompanySignatoryStatus::Invited {
                    ts: timestamp,
                    inviter: full_identity.identity.node_id.clone(),
                },
            });
        }

        self.store.update(id, &company).await?;

        let previous_block = company_chain.get_latest_block();
        let new_block = CompanyBlock::create_block_for_invite_signatory(
            id.to_owned(),
            previous_block,
            &CompanyInviteSignatoryBlockData {
                invitee: signatory_node_id.clone(),
                inviter: full_identity.identity.node_id.clone(),
                t: SignatoryType::Solo,
            },
            &full_identity.key_pair,
            &company_keys,
            &signatory_node_id.pub_key(),
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        let mut identity_chain = self.identity_blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: full_identity.identity.node_id.clone(),
            op: IdentityOpCode::InviteSignatory,
            keys: full_identity.key_pair.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_identity_block = identity_chain.get_latest_block();
        let new_identity_block = IdentityBlock::create_block_for_invite_signatory(
            previous_identity_block,
            &IdentityInviteSignatoryBlockData {
                company_id: id.to_owned(),
                block_hash: new_block.hash.clone(),
                block_id: new_block.id,
                signatory: signatory_node_id.clone(),
            },
            &full_identity.key_pair,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;
        self.validate_and_add_block(id, &mut company_chain, new_block.clone())
            .await?;
        self.populate_block(
            &company,
            &company_chain,
            &company_keys,
            Some(signatory_node_id.clone()),
        )
        .await?;

        self.validate_and_add_identity_block(&mut identity_chain, new_identity_block.clone())
            .await?;
        self.transport_service
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(
                &full_identity.identity.node_id,
                &new_identity_block,
                &full_identity.key_pair,
            ))
            .await?;

        debug!(
            "added signatory {} to company with id: {id}",
            signatory_node_id
        );

        Ok(())
    }

    async fn remove_signatory(
        &self,
        id: &NodeId,
        signatory_node_id: NodeId,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!(
            "removing signatory {} from company with id: {id}",
            signatory_node_id
        );
        validate_node_id_network(id)?;
        validate_node_id_network(&signatory_node_id)?;
        if !self.store.exists(id).await {
            return Err(super::Error::NotFound);
        }

        let full_identity = self.identity_store.get_full().await?;
        // only non-anon identities can remove signatories
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }
        let mut company = self.store.get(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;
        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: full_identity.identity.node_id.clone(),
            op: CompanyOpCode::RemoveSignatory,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: Some(signatory_node_id.clone()),
            identity_proof_data: None,
        }
        .validate()?;

        let signatory_idx = company
            .signatories
            .iter()
            .position(|s| s.node_id == signatory_node_id)
            .ok_or_else(|| {
                super::Error::Validation(
                    ProtocolValidationError::NotASignatory(signatory_node_id.to_string()).into(),
                )
            })?;

        company.signatories[signatory_idx].status = CompanySignatoryStatus::Removed {
            ts: timestamp,
            remover: full_identity.identity.node_id.clone(),
        };

        if full_identity.identity.node_id == signatory_node_id {
            info!("Removing self from company {id} - setting status to inactive");
            company.status = CompanyStatus::None;
        }

        self.store.update(id, &company).await?;

        let previous_block = company_chain.get_latest_block();
        let new_block = CompanyBlock::create_block_for_remove_signatory(
            id.to_owned(),
            previous_block,
            &CompanyRemoveSignatoryBlockData {
                removee: signatory_node_id.clone(),
                remover: full_identity.identity.node_id.clone(),
            },
            &full_identity.key_pair,
            &company_keys,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        let mut identity_chain = self.identity_blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: full_identity.identity.node_id.clone(),
            op: IdentityOpCode::RemoveSignatory,
            keys: full_identity.key_pair.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_identity_block = identity_chain.get_latest_block();
        let new_identity_block = IdentityBlock::create_block_for_remove_signatory(
            previous_identity_block,
            &IdentityRemoveSignatoryBlockData {
                company_id: id.to_owned(),
                block_hash: new_block.hash.clone(),
                block_id: new_block.id,
                signatory: signatory_node_id.clone(),
            },
            &full_identity.key_pair,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.validate_and_add_block(id, &mut company_chain, new_block.clone())
            .await?;
        self.populate_block(&company, &company_chain, &company_keys, None)
            .await?;

        self.validate_and_add_identity_block(&mut identity_chain, new_identity_block.clone())
            .await?;
        self.transport_service
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(
                &full_identity.identity.node_id,
                &new_identity_block,
                &full_identity.key_pair,
            ))
            .await?;

        if full_identity.identity.node_id == signatory_node_id {
            info!("Removed self from company {id} - deleting company chain");
            if let Err(e) = self.company_blockchain_store.remove(id).await {
                error!("Could not delete local company chain for {id}: {e}");
            }
            // If the current active identity is the company we're removed from - set to personal identity
            if let Ok(Some(active_node_id)) = self
                .identity_store
                .get_current_identity()
                .await
                .map(|i| i.company)
                && &active_node_id == id
                && let Err(e) = self
                    .identity_store
                    .set_current_identity(&ActiveIdentityState {
                        personal: full_identity.identity.node_id,
                        company: None,
                    })
                    .await
            {
                error!(
                    "Couldn't set active identity to personal after removing self from company: {e}"
                );
            }
        }
        debug!(
            "removed signatory {} to company with id: {id}",
            signatory_node_id
        );

        Ok(())
    }

    async fn open_and_decrypt_file(
        &self,
        company: Company,
        id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>> {
        let full_identity = self.identity_store.get_full().await?;
        // if we're anon - we don't return files for companies
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::NotFound);
        }
        debug!("getting file {file_name} for company with id: {id}",);
        validate_node_id_network(id)?;
        let mut file = None;
        if let Some(logo_file) = company.logo_file
            && &logo_file.name == file_name
        {
            file = Some(logo_file);
        }

        if let Some(proof_of_registration_file) = company.proof_of_registration_file
            && &proof_of_registration_file.name == file_name
        {
            file = Some(proof_of_registration_file);
        }

        if let Some(file) = file {
            let file_bytes = download_file_with_fallback(
                self.file_upload_client.as_ref(),
                Some(&self.file_reference_store),
                Some(&self.transport_service),
                &configured_blossom_servers(&get_config().nostr_config),
                &file.hash,
                &file.nostr_hash,
            )
            .await?;
            let decrypted = crypto::decrypt_ecies(&file_bytes, private_key)?;
            let file_hash = Sha256Hash::from_bytes(&decrypted);
            if file_hash != file.hash {
                error!("Hash for company file {file_name} did not match uploaded file");
                return Err(super::Error::NotFound);
            }
            Ok(decrypted.to_vec())
        } else {
            Err(super::Error::NotFound)
        }
    }

    async fn share_contact_details(&self, share_to: &NodeId, company_id: NodeId) -> Result<()> {
        let company_keys = self.store.get_key_pair(&company_id).await?;
        let derived_keys = company_keys.derive_company_keypair()?;
        let keys = BcrKeys::from_private_key(&derived_keys.secret_key());
        self.transport_service
            .contact_transport()
            .share_contact_details_keys(share_to, &company_id, &keys, None)
            .await?;
        Ok(())
    }

    async fn dev_mode_get_full_company_chain(
        &self,
        id: &NodeId,
    ) -> Result<Vec<CompanyBlockPlaintextWrapper>> {
        // if dev mode is off - we return an error
        if !get_config().dev_mode_config.on {
            error!("Called dev mode operation with dev mode disabled - please enable!");
            return Err(Error::Validation(ValidationError::InvalidOperation));
        }
        validate_node_id_network(id)?;

        // if there is no such company, we return an error
        if !self.store.exists(id).await {
            return Err(Error::NotFound);
        }

        let chain = self.company_blockchain_store.get_chain(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;

        let plaintext_chain = chain
            .get_chain_with_plaintext_block_data(&company_keys)
            .map_err(|e| Error::Protocol(e.into()))?;

        Ok(plaintext_chain)
    }

    async fn publish_contact(&self, company: &Company, keys: &BcrKeys) -> Result<()> {
        debug!("Publishing our company contact to nostr profile");
        let relays = get_config().nostr_config.relays.clone();
        let mint_url = Some(get_config().mint_config.default_mint_url.clone());
        let bcr_data = get_bcr_data(company, keys, relays.clone(), mint_url)?;
        let contact_data = NostrContactData::new(
            &company.name,
            relays,
            configured_blossom_servers(&get_config().nostr_config),
            bcr_data,
        );
        debug!("Publishing company contact data: {contact_data:?}");
        self.transport_service
            .contact_transport()
            .publish_contact(&company.id, &contact_data)
            .await?;
        self.transport_service
            .contact_transport()
            .ensure_nostr_contact(&company.id)
            .await;
        Ok(())
    }

    /// Change the email of the signatory (local identity only)
    async fn change_signatory_email(&self, id: &NodeId, email: &Email) -> Result<()> {
        debug!("updating signatory email");
        let identity = self.identity_store.get_full().await?;
        let company_keys = self.store.get_key_pair(id).await?;
        let mut company = self.store.get(id).await?;

        let Some(signatory) = company
            .signatories
            .iter_mut()
            .find(|s| s.node_id == identity.identity.node_id)
        else {
            return Err(super::Error::Validation(
                ProtocolValidationError::CallerMustBeSignatory.into(),
            ));
        };

        if let CompanySignatoryStatus::InviteAcceptedIdentityProven { ref mut data, .. } =
            signatory.status
        {
            if &data.email != email {
                data.email = email.to_owned();
            } else {
                // return early, if email didn't change
                return Ok(());
            }
        } else {
            return Err(super::Error::Validation(
                ProtocolValidationError::CallerMustBeSignatory.into(),
            ));
        }

        // check if email is confirmed
        let (proof, data) = self
            .check_confirmed_email(&identity.identity.node_id, id, email)
            .await?;

        // update signatories list in the DB
        self.store.update(id, &company).await?;
        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: identity.identity.node_id.clone(),
            op: CompanyOpCode::IdentityProof,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: Some((proof.clone(), data.clone(), None)),
        }
        .validate()?;
        // Create and populate company proof block
        self.create_identity_proof_block(
            proof,
            data,
            &company,
            &identity.key_pair,
            &company_keys,
            &mut company_chain,
            &None, // no reference block, just updating signatory email
        )
        .await?;

        debug!("updated signatory email");
        Ok(())
    }

    async fn confirm_email(&self, id: &NodeId, email: &Email) -> Result<()> {
        let identity = self.identity_store.get_full().await?;

        // use default mint URL for now, until we support multiple mints
        let mint_url = get_config().mint_config.default_mint_url.to_owned();

        self.email_client
            .register(
                &mint_url,
                &identity.identity.node_id,
                &Some(id.to_owned()),
                email,
                &identity.key_pair.get_private_key(),
            )
            .await?;

        Ok(())
    }

    async fn verify_email(&self, id: &NodeId, confirmation_code: &str) -> Result<()> {
        let identity = self.identity_store.get_full().await?;
        let node_id = identity.identity.node_id;
        let identity_keys = identity.key_pair;

        // use default mint URL for now, until we support multiple mints
        let mint_url = get_config().mint_config.default_mint_url.to_owned();
        let mint_node_id = get_config().mint_config.default_mint_node_id.to_owned();

        let (signed_proof, signed_email_identity_data) = self
            .email_client
            .confirm(
                &mint_url,
                &mint_node_id,
                &node_id,
                &Some(id.to_owned()),
                confirmation_code,
                &identity_keys.get_private_key(),
            )
            .await?;

        self.store
            .set_email_confirmation(id, &signed_proof, &signed_email_identity_data)
            .await?;

        let email_preferences_link = self
            .email_client
            .get_email_preferences_link(
                &mint_url,
                &node_id,
                &Some(id.to_owned()),
                &identity_keys.get_private_key(),
            )
            .await?;
        self.email_notification_store
            .add_email_preferences_link_for_node_id(&email_preferences_link, id)
            .await?;

        Ok(())
    }

    async fn get_email_confirmations(
        &self,
        id: &NodeId,
    ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>> {
        let email_confirmations = self.store.get_email_confirmations(id).await?;
        Ok(email_confirmations)
    }

    async fn get_active_company_invites(&self) -> Result<Vec<Company>> {
        let full_identity = self.identity_store.get_full().await?;
        // if we're anon - we don't return active invites
        if full_identity.identity.t == IdentityType::Anon {
            return Ok(vec![]);
        }

        let invites = self.store.get_active_company_invites().await?;
        let companies: Vec<Company> = invites
            .into_iter()
            .map(|(_id, (company, _keys))| company)
            .collect();
        Ok(companies)
    }

    async fn accept_company_invite(
        &self,
        id: &NodeId,
        email: &Email,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!("accepting invite to company with id: {id}");
        validate_node_id_network(id)?;
        if !self.store.exists(id).await {
            return Err(super::Error::NotFound);
        }
        let full_identity = self.identity_store.get_full().await?;
        // Can only accept if we're not anon
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }
        let our_node_id = full_identity.identity.node_id.clone();
        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        let mut company = self.store.get(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: our_node_id.clone(),
            op: CompanyOpCode::SignatoryAcceptInvite,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: None,
        }
        .validate()?;

        // check if we were invited
        let Some(signatory) = company
            .signatories
            .iter_mut()
            .find(|s| s.node_id == our_node_id)
        else {
            return Err(Error::Validation(
                ProtocolValidationError::NotInvitedAsSignatory.into(),
            ));
        };

        // check if email is confirmed
        let (proof, data) = self.check_confirmed_email(&our_node_id, id, email).await?;

        signatory.status = CompanySignatoryStatus::InviteAcceptedIdentityProven {
            ts: timestamp,
            proof: proof.clone(),
            data: data.clone(),
        }; // invite accepted and identity proven
        company.status = CompanyStatus::Active; // we're now active in the company
        // update signatories list in the DB
        self.store.update(id, &company).await?;

        // Ensure the company identity is registered with the transport layer before
        // broadcasting chain events, in case it was lost (e.g. after restart).
        self.transport_service
            .add_identity(id, &company_keys)
            .await?;

        // Ensure all company signatories and the company itself are in our nostr contacts
        // so that we can receive and decrypt messages from them.
        for signatory in company.signatories.iter() {
            if signatory.node_id != our_node_id {
                self.transport_service
                    .contact_transport()
                    .ensure_nostr_contact(&signatory.node_id)
                    .await;
            }
        }
        self.transport_service
            .contact_transport()
            .ensure_nostr_contact(id)
            .await;

        // Process any historical bill invites that were sent to this company
        // before we joined, so we can construct those bills in our store.
        let transport = self.transport_service.clone();
        let company_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = transport
                .process_company_historical_bill_invites(&company_id)
                .await
            {
                error!(
                    "Failed to process historical bill invites for company {}: {}",
                    company_id, e
                );
            }
        });

        // company block
        let previous_block = company_chain.get_latest_block();
        let new_block = CompanyBlock::create_block_for_accept_signatory_invite(
            id.to_owned(),
            previous_block,
            &CompanySignatoryAcceptInviteBlockData {
                accepter: our_node_id.clone(),
            },
            &full_identity.key_pair,
            &company_keys,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        // identity block
        let mut identity_chain = self.identity_blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: full_identity.identity.node_id.clone(),
            op: IdentityOpCode::AcceptSignatoryInvite,
            keys: full_identity.key_pair.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_identity_block = identity_chain.get_latest_block();
        let new_identity_block = IdentityBlock::create_block_for_accept_signatory_invite(
            previous_identity_block,
            &IdentityAcceptSignatoryInviteBlockData {
                company_id: id.to_owned(),
                block_hash: new_block.hash.clone(),
                block_id: new_block.id,
            },
            &full_identity.key_pair,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.validate_and_add_block(id, &mut company_chain, new_block.clone())
            .await?;
        self.populate_block(&company, &company_chain, &company_keys, None)
            .await?;

        self.validate_and_add_identity_block(&mut identity_chain, new_identity_block.clone())
            .await?;
        self.transport_service
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(
                &our_node_id,
                &new_identity_block,
                &full_identity.key_pair,
            ))
            .await?;

        let reference_block = new_block.id();
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: our_node_id.clone(),
            op: CompanyOpCode::IdentityProof,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: Some((proof.clone(), data.clone(), Some(reference_block))),
        }
        .validate()?;
        // Create and populate company proof block
        self.create_identity_proof_block(
            proof,
            data,
            &company,
            &full_identity.key_pair,
            &company_keys,
            &mut company_chain,
            &Some(reference_block),
        )
        .await?;

        debug!("accepted invite to company with id: {id}");
        Ok(())
    }

    async fn reject_company_invite(&self, id: &NodeId, timestamp: Timestamp) -> Result<()> {
        debug!("rejecting invite to company with id: {id}");
        validate_node_id_network(id)?;
        if !self.store.exists(id).await {
            return Err(super::Error::NotFound);
        }

        let full_identity = self.identity_store.get_full().await?;
        // Can only reject if we're not anon
        if full_identity.identity.t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }
        let our_node_id = full_identity.identity.node_id.clone();
        let mut company_chain = self.company_blockchain_store.get_chain(id).await?;
        let mut company = self.store.get(id).await?;
        let company_keys = self.store.get_key_pair(id).await?;
        CompanyValidateActionData {
            blockchain: company_chain.clone(),
            company_id: id.to_owned(),
            signer_node_id: our_node_id.clone(),
            op: CompanyOpCode::SignatoryRejectInvite,
            company_keys: company_keys.clone(),
            invitee: None,
            removee: None,
            identity_proof_data: None,
        }
        .validate()?;

        let Some(signatory) = company
            .signatories
            .iter_mut()
            .find(|s| s.node_id == our_node_id)
        else {
            return Err(Error::Validation(
                ProtocolValidationError::NotInvitedAsSignatory.into(),
            ));
        };

        signatory.status = CompanySignatoryStatus::InviteRejected { ts: timestamp }; // invite rejected
        company.status = CompanyStatus::None; // not interested in the company anymore
        // update signatories list in the DB
        self.store.update(id, &company).await?;

        // company block
        let previous_block = company_chain.get_latest_block();
        let new_block = CompanyBlock::create_block_for_reject_signatory_invite(
            id.to_owned(),
            previous_block,
            &CompanySignatoryRejectInviteBlockData {
                rejecter: our_node_id.clone(),
            },
            &full_identity.key_pair,
            &company_keys,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        // identity block
        let mut identity_chain = self.identity_blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: full_identity.identity.node_id.clone(),
            op: IdentityOpCode::RejectSignatoryInvite,
            keys: full_identity.key_pair.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_identity_block = identity_chain.get_latest_block();
        let new_identity_block = IdentityBlock::create_block_for_reject_signatory_invite(
            previous_identity_block,
            &IdentityRejectSignatoryInviteBlockData {
                company_id: id.to_owned(),
                block_hash: new_block.hash.clone(),
                block_id: new_block.id,
            },
            &full_identity.key_pair,
            timestamp,
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.validate_and_add_block(id, &mut company_chain, new_block.clone())
            .await?;
        self.populate_block(&company, &company_chain, &company_keys, None)
            .await?;

        self.validate_and_add_identity_block(&mut identity_chain, new_identity_block.clone())
            .await?;
        self.transport_service
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(
                &our_node_id,
                &new_identity_block,
                &full_identity.key_pair,
            ))
            .await?;

        debug!("rejected invite to company with id: {id}");
        Ok(())
    }

    async fn locally_hide_signatory(&self, id: &NodeId, signatory_node_id: &NodeId) -> Result<()> {
        validate_node_id_network(id)?;
        validate_node_id_network(signatory_node_id)?;
        if !self.store.exists(id).await {
            return Err(super::Error::NotFound);
        }

        let full_identity = self.identity_store.get_full().await?;
        let company = self.store.get(id).await?;
        if !company.is_authorized_signer(&full_identity.identity.node_id) {
            return Err(super::Error::Validation(
                ProtocolValidationError::CallerMustBeSignatory.into(),
            ));
        }

        if let Some(signatory) = company
            .signatories
            .iter()
            .find(|s| &s.node_id == signatory_node_id)
            && matches!(
                signatory.status,
                CompanySignatoryStatus::Removed { .. }
                    | CompanySignatoryStatus::InviteRejected { .. }
            )
        {
            self.store
                .set_local_signatory_override(
                    id,
                    signatory_node_id,
                    LocalSignatoryOverrideStatus::Hidden,
                )
                .await?;
        } else {
            return Err(super::Error::Validation(
                ProtocolValidationError::NotARemovedOrRejectedSignatory.into(),
            ));
        }

        Ok(())
    }

    async fn filter_out_locally_hidden_signatories(
        &self,
        company_id: &NodeId,
        signatories: &mut Vec<CompanySignatory>,
    ) -> Result<()> {
        let local_overrides = self.store.get_local_signatory_overrides(company_id).await?;
        if local_overrides.is_empty() {
            return Ok(());
        }
        let node_ids_to_filter: HashSet<NodeId> =
            local_overrides.into_iter().map(|s| s.node_id).collect();

        // filter out node ids that should be hidden, but only if they are rejected, or removed
        signatories.retain(|sig| {
            !(node_ids_to_filter.contains(&sig.node_id)
                && matches!(
                    sig.status,
                    CompanySignatoryStatus::Removed { .. }
                        | CompanySignatoryStatus::InviteRejected { .. }
                ))
        });
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{
        external::{email::MockEmailClientApi, file_storage::MockFileStorageClientApi},
        service::{
            contact_service::tests::{get_baseline_contact, get_baseline_nostr_contact},
            transport_service::MockTransportServiceApi,
        },
        tests::tests::{
            MockCompanyChainStoreApiMock, MockCompanyStoreApiMock, MockContactStoreApiMock,
            MockEmailNotificationStoreApiMock, MockFileReferenceStoreApiMock,
            MockFileUploadStoreApiMock, MockIdentityChainStoreApiMock, MockIdentityStoreApiMock,
            MockNostrContactStore, empty_address, empty_identity, empty_other_identity,
            node_id_test, node_id_test_another, node_id_test_other, node_id_test_other2,
            private_key_test, private_key_test_another, signed_identity_proof_test,
            signed_other_identity_proof_test, test_ts,
        },
        util::get_uuid_v4,
    };
    use bcr_ebill_core::{
        application::{company::LocalSignatoryOverride, identity::IdentityWithAll},
        protocol::{
            Country,
            blockchain::identity::{IdentityBlockchain, IdentityProofBlockData},
            file_reference::FileReference,
        },
    };
    use bitcoin::hashes::sha256::Hash as Sha256HexHash;
    use mockall::predicate::eq;
    use std::{collections::HashMap, str::FromStr};

    fn get_service(
        mock_storage: MockCompanyStoreApiMock,
        mock_file_upload_storage: MockFileUploadStoreApiMock,
        mock_file_upload_client: MockFileStorageClientApi,
        mock_file_reference_store: MockFileReferenceStoreApiMock,
        mock_identity_storage: MockIdentityStoreApiMock,
        mock_contacts_storage: MockContactStoreApiMock,
        mock_nostr_contact_store: MockNostrContactStore,
        mock_identity_chain_storage: MockIdentityChainStoreApiMock,
        mock_company_chain_storage: MockCompanyChainStoreApiMock,
        transport_service: MockTransportServiceApi,
        mock_email_client: MockEmailClientApi,
        mock_email_notification_store: MockEmailNotificationStoreApiMock,
    ) -> CompanyService {
        CompanyService::new(
            Arc::new(mock_storage),
            Arc::new(mock_file_upload_storage),
            Arc::new(mock_file_upload_client),
            Arc::new(mock_file_reference_store),
            Arc::new(mock_identity_storage),
            Arc::new(mock_contacts_storage),
            Arc::new(mock_nostr_contact_store),
            Arc::new(mock_identity_chain_storage),
            Arc::new(mock_company_chain_storage),
            Arc::new(transport_service),
            Arc::new(mock_email_client),
            Arc::new(mock_email_notification_store),
        )
    }

    fn get_storages() -> (
        MockCompanyStoreApiMock,
        MockFileUploadStoreApiMock,
        MockFileStorageClientApi,
        MockFileReferenceStoreApiMock,
        MockIdentityStoreApiMock,
        MockContactStoreApiMock,
        MockIdentityChainStoreApiMock,
        MockCompanyChainStoreApiMock,
        MockTransportServiceApi,
        MockNostrContactStore,
        MockEmailClientApi,
        MockEmailNotificationStoreApiMock,
    ) {
        (
            MockCompanyStoreApiMock::new(),
            MockFileUploadStoreApiMock::new(),
            MockFileStorageClientApi::new(),
            MockFileReferenceStoreApiMock::new(),
            MockIdentityStoreApiMock::new(),
            MockContactStoreApiMock::new(),
            MockIdentityChainStoreApiMock::new(),
            MockCompanyChainStoreApiMock::new(),
            MockTransportServiceApi::new(),
            MockNostrContactStore::new(),
            MockEmailClientApi::new(),
            MockEmailNotificationStoreApiMock::new(),
        )
    }

    pub fn get_baseline_company_data() -> (NodeId, (Company, BcrKeys)) {
        (
            node_id_test(),
            (
                Company {
                    id: node_id_test(),
                    name: Name::new("some_name").unwrap(),
                    country_of_registration: Some(Country::AT),
                    city_of_registration: Some(City::new("Vienna").unwrap()),
                    postal_address: empty_address(),
                    email: Email::new("company@example.com").unwrap(),
                    registration_number: Some(Identification::new("some_number").unwrap()),
                    registration_date: Some(Date::new("2012-01-01").unwrap()),
                    proof_of_registration_file: None,
                    logo_file: None,
                    signatories: vec![get_valid_activated_signatory(&node_id_test_another())],
                    creation_time: test_ts(),
                    status: CompanyStatus::Active,
                },
                BcrKeys::from_private_key(&private_key_test()),
            ),
        )
    }

    pub fn get_baseline_company() -> Company {
        get_baseline_company_data().1.0
    }

    pub fn get_valid_company_block() -> CompanyBlock {
        get_valid_company_chain().get_latest_block().to_owned()
    }

    pub fn get_valid_activated_signatory(node_id: &NodeId) -> CompanySignatory {
        let (proof, data) = signed_other_identity_proof_test();
        CompanySignatory {
            t: SignatoryType::Solo,
            node_id: node_id.to_owned(),
            status: CompanySignatoryStatus::InviteAcceptedIdentityProven {
                ts: test_ts(),
                data,
                proof,
            },
        }
    }

    pub fn get_valid_company_chain() -> CompanyBlockchain {
        let (id, (company, company_keys)) = get_baseline_company_data();
        let mut chain = CompanyBlockchain::new(
            &CompanyCreateBlockData {
                id,
                name: company.name,
                country_of_registration: company.country_of_registration,
                city_of_registration: company.city_of_registration,
                postal_address: company.postal_address,
                email: company.email,
                registration_number: company.registration_number,
                registration_date: company.registration_date,
                proof_of_registration_file: company.proof_of_registration_file,
                logo_file: company.logo_file,
                creation_time: company.creation_time,
                creator: node_id_test_another(),
            },
            &BcrKeys::from_private_key(&private_key_test_another()),
            &company_keys,
            test_ts() - 10,
        )
        .unwrap();
        let (proof, mut data) = signed_other_identity_proof_test();
        data.company_node_id = Some(node_id_test());
        let identity_proof_block = CompanyBlock::create_block_for_identity_proof(
            node_id_test(),
            chain.get_latest_block(),
            &CompanyIdentityProofBlockData {
                proof,
                data,
                reference_block: Some(chain.get_latest_block().id()),
            },
            &BcrKeys::from_private_key(&private_key_test_another()),
            &company_keys,
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

    fn add_other_invite_signatory_block(mut chain: CompanyBlockchain) -> CompanyBlockchain {
        let block = CompanyBlock::create_block_for_invite_signatory(
            node_id_test(),
            chain.get_latest_block(),
            &CompanyInviteSignatoryBlockData {
                invitee: node_id_test_other(),
                inviter: node_id_test_another(),
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

    fn add_accept_signatory_block(mut chain: CompanyBlockchain) -> CompanyBlockchain {
        let block = CompanyBlock::create_block_for_accept_signatory_invite(
            node_id_test(),
            chain.get_latest_block(),
            &CompanySignatoryAcceptInviteBlockData {
                accepter: node_id_test_another(),
            },
            &BcrKeys::from_private_key(&private_key_test_another()),
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts() - 7,
        )
        .unwrap();
        assert!(chain.try_add_block(block));
        assert!(chain.is_chain_valid());
        chain
    }

    fn add_identity_proof_block(mut chain: CompanyBlockchain) -> CompanyBlockchain {
        let (proof, mut data) = signed_identity_proof_test();
        data.company_node_id = Some(node_id_test());
        let block = CompanyBlock::create_block_for_identity_proof(
            node_id_test(),
            chain.get_latest_block(),
            &CompanyIdentityProofBlockData {
                proof,
                data,
                reference_block: Some(chain.get_latest_block().id()),
            },
            &BcrKeys::from_private_key(&private_key_test_another()),
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts() - 6,
        )
        .unwrap();
        assert!(chain.try_add_block(block));
        assert!(chain.is_chain_valid());
        chain
    }

    fn add_identity_proof_block_for_identity(mut chain: IdentityBlockchain) -> IdentityBlockchain {
        let (proof, data) = signed_other_identity_proof_test();
        let block = IdentityBlock::create_block_for_identity_proof(
            chain.get_latest_block(),
            &IdentityProofBlockData { proof, data },
            &BcrKeys::from_private_key(&private_key_test_another()),
            test_ts() - 6,
        )
        .unwrap();
        assert!(chain.try_add_block(block));
        assert!(chain.is_chain_valid());
        chain
    }

    pub fn get_valid_identity_chain() -> IdentityBlockchain {
        let chain = IdentityBlockchain::new(
            &empty_other_identity().into(),
            &BcrKeys::from_private_key(&private_key_test_another()),
            test_ts() - 7,
        )
        .unwrap();
        add_identity_proof_block_for_identity(chain)
    }

    #[tokio::test]
    async fn get_list_of_companies_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_get_all().returning(|| {
            let mut map = HashMap::new();
            let company_data = get_baseline_company_data();
            map.insert(company_data.0, company_data.1);
            Ok(map)
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service.get_list_of_companies().await;
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert_eq!(res.as_ref().unwrap()[0].id, node_id_test());
    }

    #[tokio::test]
    async fn get_list_of_invites() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_get_active_company_invites().returning(|| {
            let mut map = HashMap::new();
            let company_data = get_baseline_company_data();
            map.insert(company_data.0, company_data.1);
            Ok(map)
        });
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service.get_active_company_invites().await;
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 1);
        assert_eq!(res.as_ref().unwrap()[0].id, node_id_test());
    }

    #[tokio::test]
    async fn get_list_of_companies_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_get_all().returning(|| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service.get_list_of_companies().await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn get_company_by_id_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage
            .expect_get()
            .returning(|_| Ok(get_baseline_company_data().1.0));
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service.get_company_by_id(&node_id_test()).await;
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().id, node_id_test());
    }

    #[tokio::test]
    async fn get_company_by_id_fails_if_company_doesnt_exist() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| false);
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service.get_company_by_id(&node_id_test()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn get_company_by_id_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service.get_company_by_id(&node_id_test()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn create_company_baseline() {
        crate::tests::tests::init_test_cfg();
        let (
            mut storage,
            mut file_upload_store,
            mut file_upload_client,
            mut file_reference_store,
            mut identity_store,
            mut contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        contact_store.expect_get().returning(|_| Ok(None));
        contact_store.expect_insert().returning(|_, _| Ok(()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        file_upload_client.expect_upload().returning(|_, _| {
            Ok(bitcoin::hashes::sha256::Hash::from_str(
                "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
            )
            .unwrap())
        });
        storage.expect_save_key_pair().returning(|_, _| Ok(()));
        storage.expect_exists().returning(|_| false);
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        storage.expect_insert().returning(|_| Ok(()));
        storage.expect_get_email_confirmations().returning(|_| {
            let (proof, mut data) = signed_other_identity_proof_test();
            data.company_node_id = Some(node_id_test());
            Ok(vec![(proof, data)])
        });
        identity_store.expect_get_full().returning(|| {
            let mut identity = empty_other_identity();
            identity.nostr_relays = vec![url::Url::parse("ws://localhost:8080").unwrap()];
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        file_upload_store
            .expect_read_temp_upload_file()
            .returning(|_| {
                Ok((
                    Name::new("some_file").unwrap(),
                    "hello_world".as_bytes().to_vec(),
                ))
            });
        file_upload_store
            .expect_remove_temp_upload_folder()
            .returning(|_| Ok(()));
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));
        file_reference_store.expect_get().returning(|_| Ok(None));
        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(true));
        file_reference_store
            .expect_upsert()
            .returning(|_, _, _, _, _, _| {
                Ok(FileReference::new(
                    Sha256Hash::from_bytes(b"test"),
                    bitcoin::hashes::sha256::Hash::from_str(
                        "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                    )
                    .unwrap(),
                    None,
                ))
            })
            .times(2);
        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()))
            .times(1);
        transport
            .expect_add_identity()
            .returning(|_, _| Ok(()))
            .times(1);
        transport.expect_on_block_transport(|t| {
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .times(2);
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .times(1);
        });
        transport.expect_on_contact_transport(|c| {
            c.expect_publish_contact().returning(|_, _| Ok(()));
            c.expect_ensure_nostr_contact().returning(|_| ());
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .create_company(
                node_id_test(),
                Name::new("name").unwrap(),
                None,
                None,
                empty_address(),
                Email::new("company@example.com").unwrap(),
                None,
                None,
                None,
                Some(get_uuid_v4()),
                Email::new("test@example.com").unwrap(),
                test_ts(),
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn create_company_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            mut notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_save_key_pair().returning(|_, _| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        storage.expect_insert().returning(|_| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        storage
            .expect_get_email_confirmations()
            .returning(|_| Ok(vec![signed_identity_proof_test()]));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        notification
            .expect_add_identity()
            .returning(|_, _| Ok(()))
            .times(1);
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .create_company(
                node_id_test(),
                Name::new("name").unwrap(),
                Some(Country::AT),
                Some(City::new("Vienna").unwrap()),
                empty_address(),
                Email::new("company@example.com").unwrap(),
                Some(Identification::new("some_number").unwrap()),
                Some(Date::new("2012-01-01").unwrap()),
                None,
                None,
                Email::new("test@example.com").unwrap(),
                test_ts(),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn edit_company_fails_if_company_doesnt_exist() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| false);
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .edit_company(
                &node_id_test(),
                Some(Name::new("name").unwrap()),
                Some(Email::new("company@example.com").unwrap()),
                None,
                None,
                EditOptionalFieldMode::Ignore,
                None,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                test_ts(),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn edit_company_baseline() {
        crate::tests::tests::init_test_cfg();
        let (
            mut storage,
            mut file_upload_store,
            mut file_upload_client,
            mut file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();

        transport.expect_on_block_transport(|t| {
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
        });

        transport.expect_on_contact_transport(|t| {
            t.expect_publish_contact().returning(|_, _| Ok(())).once();
            t.expect_ensure_nostr_contact().returning(|_| ()).once();
        });

        storage.expect_get().returning(move |_| {
            let data = get_baseline_company_data().1.0;
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        storage.expect_exists().returning(|_| true);
        storage.expect_update().returning(|_, _| Ok(()));
        file_upload_client.expect_upload().returning(|_, _| {
            Ok(bitcoin::hashes::sha256::Hash::from_str(
                "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
            )
            .unwrap())
        });
        identity_store.expect_get_full().returning(move || {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        file_upload_store
            .expect_read_temp_upload_file()
            .returning(|_| {
                Ok((
                    Name::new("some_file").unwrap(),
                    "hello_world".as_bytes().to_vec(),
                ))
            });
        file_upload_store
            .expect_remove_temp_upload_folder()
            .returning(|_| Ok(()));
        file_reference_store.expect_get().returning(|_| Ok(None));
        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(true));
        file_reference_store
            .expect_upsert()
            .returning(|_, _, _, _, _, _| {
                Ok(FileReference::new(
                    Sha256Hash::from_bytes(b"test"),
                    bitcoin::hashes::sha256::Hash::from_str(
                        "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                    )
                    .unwrap(),
                    None,
                ))
            })
            .times(2);
        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()))
            .times(1);

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .edit_company(
                &node_id_test(),
                Some(Name::new("name").unwrap()),
                Some(Email::new("company@example.com").unwrap()),
                None,
                None,
                EditOptionalFieldMode::Ignore,
                None,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Set(get_uuid_v4()),
                EditOptionalFieldMode::Ignore,
                test_ts(),
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn edit_company_fails_if_caller_is_not_signatory() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories = vec![get_valid_activated_signatory(&node_id_test_other())];
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        identity_store.expect_get_full().returning(|| {
            let mut identity = empty_other_identity();
            identity.node_id = node_id_test_other();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .edit_company(
                &node_id_test(),
                Some(Name::new("name").unwrap()),
                Some(Email::new("company@example.com").unwrap()),
                None,
                None,
                EditOptionalFieldMode::Ignore,
                None,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                test_ts(),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn edit_company_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        let keys = BcrKeys::new();
        let node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
        let node_id_clone = node_id.clone();
        storage.expect_get().returning(move |_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories = vec![get_valid_activated_signatory(&node_id_clone)];
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        storage.expect_exists().returning(|_| true);
        storage.expect_update().returning(|_, _| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        identity_store.expect_get_full().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = node_id.clone();
            Ok(IdentityWithAll {
                identity,
                key_pair: keys.clone(),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .edit_company(
                &node_id_test(),
                Some(Name::new("name").unwrap()),
                Some(Email::new("company@example.com").unwrap()),
                None,
                None,
                EditOptionalFieldMode::Ignore,
                None,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                EditOptionalFieldMode::Ignore,
                test_ts(),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn reject_company_invite_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_update().returning(|_, _| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_invite_signatory_block(get_valid_company_chain())))
            .once();
        transport.expect_on_block_transport(|t| {
            // sends company block
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
            // sends identity block
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        let caller_keys = BcrKeys::from_private_key(&private_key_test_another());
        let caller_keys_clone = caller_keys.clone();
        let caller_node_id = NodeId::new(caller_keys.pub_key(), bitcoin::Network::Testnet);
        let caller_node_id_clone = caller_node_id.clone();
        storage.expect_get().returning(move |_| {
            let mut data = get_baseline_company_data().1.0;
            let mut sig = get_valid_activated_signatory(&caller_node_id_clone.clone());
            sig.status = CompanySignatoryStatus::Invited {
                ts: test_ts(),
                inviter: node_id_test(),
            };
            data.signatories = vec![sig];
            Ok(data)
        });
        identity_store.expect_get_full().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = caller_node_id.clone();
            Ok(IdentityWithAll {
                identity,
                key_pair: caller_keys_clone.clone(),
            })
        });
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .reject_company_invite(&node_id_test(), test_ts())
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn accept_company_invite_baseline() {
        crate::tests::tests::init_test_cfg();
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_update().returning(|_, _| Ok(()));
        storage.expect_get_email_confirmations().returning(|_| {
            let (proof, mut data) = signed_identity_proof_test();
            data.node_id = node_id_test_another();
            data.company_node_id = Some(node_id_test());
            Ok(vec![(proof, data)])
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_invite_signatory_block(get_valid_company_chain())))
            .once();
        transport.expect_on_block_transport(|t| {
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .times(2);
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        let caller_keys = BcrKeys::from_private_key(&private_key_test_another());
        let caller_keys_clone = caller_keys.clone();
        let caller_node_id = NodeId::new(caller_keys.pub_key(), bitcoin::Network::Testnet);
        let caller_node_id_clone = caller_node_id.clone();
        storage.expect_get().returning(move |_| {
            let mut data = get_baseline_company_data().1.0;
            let mut sig = get_valid_activated_signatory(&caller_node_id_clone);
            sig.status = CompanySignatoryStatus::Invited {
                ts: test_ts(),
                inviter: node_id_test(),
            };
            data.signatories = vec![sig];
            Ok(data)
        });
        identity_store.expect_get_full().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = caller_node_id.clone();
            Ok(IdentityWithAll {
                identity,
                key_pair: caller_keys_clone.clone(),
            })
        });
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));
        transport
            .expect_add_identity()
            .returning(|_, _| Ok(()))
            .times(1);
        transport.expect_on_contact_transport(|t| {
            t.expect_ensure_nostr_contact().returning(|_| ()).times(1);
        });
        transport
            .expect_process_company_historical_bill_invites()
            .returning(|_| Ok(()))
            .times(1);

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .accept_company_invite(
                &node_id_test(),
                &Email::new("test@example.com").unwrap(),
                test_ts(),
            )
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn invite_signatory_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        let signatory_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        storage.expect_exists().returning(|_| true);
        storage.expect_update().returning(|_, _| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        transport.expect_on_block_transport(|t| {
            // sends company block
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
            // sends identity block
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        let signatory_node_id_clone = signatory_node_id.clone();
        contact_store.expect_get_map().returning(move || {
            let mut map = HashMap::new();
            let mut contact = get_baseline_contact();
            contact.node_id = signatory_node_id_clone.clone();
            map.insert(signatory_node_id_clone.clone(), contact);
            Ok(map)
        });
        let caller_keys = BcrKeys::from_private_key(&private_key_test_another());
        let caller_keys_clone = caller_keys.clone();
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        storage.expect_get().returning(move |_| {
            let data = get_baseline_company_data().1.0;
            Ok(data)
        });
        identity_store.expect_get_full().returning(move || {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: caller_keys_clone.clone(),
            })
        });
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), signatory_node_id, test_ts())
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn invite_signatory_fails_if_signatory_in_contacts_but_not_a_person() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        let signatory_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories = vec![get_valid_activated_signatory(&node_id_test())];
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        let signatory_node_id_clone = signatory_node_id.clone();
        contact_store.expect_get_map().returning(move || {
            let mut map = HashMap::new();
            let mut contact = get_baseline_contact();
            contact.node_id = signatory_node_id_clone.clone();
            contact.t = ContactType::Company;
            map.insert(signatory_node_id_clone.clone(), contact);
            Ok(map)
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        identity_store.expect_get_full().returning(|| {
            let identity = empty_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::new(),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), signatory_node_id, test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn invite_signatory_fails_if_signatory_not_in_contacts() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories = vec![get_valid_activated_signatory(&node_id_test())];
            Ok(data)
        });
        contact_store
            .expect_get_map()
            .returning(|| Ok(HashMap::new()));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::new(),
            })
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn invite_signatory_fails_if_company_doesnt_exist() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| false);
        identity_store.expect_get_full().returning(|| {
            let identity = empty_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::new(),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn invite_signatory_fails_if_signatory_is_already_signatory() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        identity_store.expect_get_full().returning(|| {
            let identity = empty_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::new(),
            })
        });
        contact_store.expect_get_map().returning(|| {
            let mut map = HashMap::new();
            let contact = get_baseline_contact();
            map.insert(node_id_test(), contact);
            Ok(map)
        });
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories = vec![get_valid_activated_signatory(&node_id_test())];
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), node_id_test(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn invite_signatory_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        identity_store.expect_get_full().returning(|| {
            let identity = empty_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::new(),
            })
        });
        contact_store.expect_get_map().returning(|| {
            let mut map = HashMap::new();
            let contact = get_baseline_contact();
            map.insert(node_id_test(), contact);
            Ok(map)
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(get_valid_company_chain()))
            .once();
        storage.expect_update().returning(|_, _| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        storage
            .expect_get()
            .returning(|_| Ok(get_baseline_company_data().1.0));
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .invite_signatory(&node_id_test(), node_id_test(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn remove_signatory_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test_other()));
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        storage.expect_update().returning(|_, _| Ok(()));
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));

        transport.expect_on_block_transport(|t| {
            // sends identity block
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
            // sends company block
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_other_invite_signatory_block(get_valid_company_chain())))
            .once();

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn remove_signatory_fails_if_company_doesnt_exist() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| false);
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn remove_signatory_removing_self_removes_company() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| {
                Ok(add_identity_proof_block(add_accept_signatory_block(
                    add_other_invite_signatory_block(get_valid_company_chain()),
                )))
            })
            .once();
        transport.expect_on_block_transport(|t| {
            // sends company block
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
            // sends identity block
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        company_chain_store.expect_remove().returning(|_| Ok(()));
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(move |_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test()));
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test_other()));

            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(move || {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        identity_store.expect_get_current_identity().returning(|| {
            Ok(ActiveIdentityState {
                personal: node_id_test(),
                company: None,
            })
        });
        storage.expect_update().returning(|_, _| Ok(()));
        storage.expect_remove().returning(|_| Ok(()));
        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn remove_signatory_fails_if_signatory_is_not_in_company() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test()));
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test_other()));

            Ok(data)
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_invite_signatory_block(get_valid_company_chain())))
            .once();
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test_other2(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn remove_signatory_fails_on_last_signatory() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let data = get_baseline_company_data().1.0;
            Ok(data)
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_invite_signatory_block(get_valid_company_chain())))
            .once();
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn remove_signatory_allows_removing_invited_signatory_when_only_one_proven() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            mut identity_chain_store,
            mut company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();

        company_chain_store
            .expect_get_latest_block()
            .returning(|_| Ok(get_valid_company_block()));
        company_chain_store
            .expect_add_block()
            .returning(|_, _| Ok(()));
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_other_invite_signatory_block(get_valid_company_chain())))
            .once();

        transport.expect_on_block_transport(|t| {
            t.expect_send_company_chain_events()
                .returning(|_| Ok(()))
                .once();
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });

        identity_chain_store
            .expect_get_chain()
            .returning(|| Ok(get_valid_identity_chain()));
        identity_chain_store
            .expect_add_block()
            .returning(|_| Ok(()));

        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories.push(CompanySignatory {
                t: SignatoryType::Solo,
                node_id: node_id_test_other(),
                status: CompanySignatoryStatus::Invited {
                    ts: test_ts(),
                    inviter: node_id_test(),
                },
            });
            Ok(data)
        });
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        storage.expect_update().returning(|_, _| Ok(())).once();
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service
            .remove_signatory(&node_id_test(), node_id_test_other(), test_ts())
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn remove_signatory_propagates_persistence_errors() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            mut company_chain_store,
            notification,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test()));
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test_other()));
            Ok(data)
        });
        company_chain_store
            .expect_get_chain()
            .returning(|_| Ok(add_invite_signatory_block(get_valid_company_chain())))
            .once();
        storage
            .expect_get_key_pair()
            .returning(|_| Ok(get_baseline_company_data().1.1));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        storage.expect_update().returning(|_, _| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            notification,
            email_client,
            email_notification_store,
        );
        let res = service
            .remove_signatory(&node_id_test(), node_id_test(), test_ts())
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn save_encrypt_open_decrypt_compare_hashes() {
        crate::tests::tests::init_test_cfg();
        let company_id = node_id_test();
        let file_name = Name::new("file_00000000-0000-0000-0000-000000000000.pdf").unwrap();
        let file_bytes = String::from("hello world").as_bytes().to_vec();

        let expected_encrypted =
            crypto::encrypt_ecies(&file_bytes, &node_id_test().pub_key()).unwrap();

        let (
            storage,
            file_upload_store,
            mut file_upload_client,
            mut file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            mut transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();

        file_upload_client
            .expect_upload()
            .times(1)
            .returning(|_, _| {
                Ok(bitcoin::hashes::sha256::Hash::from_str(
                    "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                )
                .unwrap())
            });

        file_upload_client
            .expect_download()
            .times(1)
            .returning(move |_, _| Ok(expected_encrypted.clone()));
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        file_reference_store.expect_get().returning(|_| Ok(None));
        file_reference_store
            .expect_add_server_urls()
            .returning(|_, _| Ok(true));
        file_reference_store
            .expect_upsert()
            .returning(|_, _, _, _, _, _| {
                Ok(FileReference::new(
                    Sha256Hash::from_bytes(b"test"),
                    bitcoin::hashes::sha256::Hash::from_str(
                        "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                    )
                    .unwrap(),
                    None,
                ))
            })
            .times(2);
        transport
            .expect_publish_file_metadata()
            .returning(|_, _, _, _, _| Ok(()));
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let file = service
            .encrypt_and_upload_file(
                &file_name,
                &file_bytes,
                &company_id,
                &node_id_test().pub_key(),
                &BcrKeys::new(),
                "test_file",
            )
            .await
            .unwrap();
        assert_eq!(
            file.hash,
            Sha256Hash::from_str("DULfJyE3WQqNxy3ymuhAChyNR3yufT88pmqvAazKFMG4")
                .expect("valid hash")
        );
        assert_eq!(file.name, file_name);

        let mut company = get_baseline_company_data().1.0;
        company.proof_of_registration_file = Some(File {
            name: file_name.to_owned(),
            hash: file.hash.clone(),
            nostr_hash: Sha256HexHash::from_str(
                "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
            )
            .unwrap(),
        });

        let decrypted = service
            .open_and_decrypt_file(company, &company_id, &file_name, &private_key_test())
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&decrypted).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn save_encrypt_propagates_upload_error() {
        let (
            storage,
            file_upload_store,
            mut file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        file_upload_client.expect_upload().returning(|_, _| {
            Err(crate::external::Error::ExternalFileStorageApi(
                crate::external::file_storage::Error::InvalidRelayUrl,
            ))
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        assert!(
            service
                .encrypt_and_upload_file(
                    &Name::new("file_name").unwrap(),
                    &[],
                    &node_id_test(),
                    &node_id_test().pub_key(),
                    &BcrKeys::new(),
                    "test_file",
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn open_decrypt_propagates_read_file_error() {
        let (
            storage,
            file_upload_store,
            mut file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        file_upload_client.expect_upload().returning(|_, _| {
            Err(crate::external::Error::ExternalFileStorageApi(
                crate::external::file_storage::Error::InvalidRelayUrl,
            ))
        });
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        assert!(
            service
                .open_and_decrypt_file(
                    get_baseline_company_data().1.0,
                    &node_id_test(),
                    &Name::new("test").unwrap(),
                    &private_key_test()
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_signatories_baseline() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            mut contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            mut nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;
            data.signatories
                .push(get_valid_activated_signatory(&node_id_test_other()));
            Ok(data)
        });

        contact_store.expect_get_map().returning(move || {
            let mut map = HashMap::new();
            let mut contact = get_baseline_contact();
            contact.node_id = node_id_test();
            map.insert(contact.node_id.clone(), contact);
            Ok(map)
        });

        identity_store
            .expect_get()
            .returning(|| Ok(empty_other_identity()));

        // should also try to look up the other signatory contact in nostr contacts
        nostr_contact_store
            .expect_by_node_ids()
            .with(eq(vec![node_id_test_other()]))
            .returning(|_| {
                let contact = get_baseline_nostr_contact();
                Ok(vec![contact])
            });

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service.list_signatories(&node_id_test()).await;
        assert!(res.is_ok());
        assert_eq!(res.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_locally_hide_signatory() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();

        storage.expect_exists().returning(|_| true);
        storage.expect_get().returning(|_| {
            let mut data = get_baseline_company_data().1.0;

            let mut sig = get_valid_activated_signatory(&node_id_test_other());
            sig.status = CompanySignatoryStatus::Removed {
                ts: test_ts(),
                remover: node_id_test(),
            };
            data.signatories.push(sig);
            Ok(data)
        });
        storage
            .expect_set_local_signatory_override()
            .returning(|_, _, _| Ok(()))
            .once();
        identity_store.expect_get_full().returning(|| {
            let identity = empty_other_identity();
            Ok(IdentityWithAll {
                identity,
                key_pair: BcrKeys::from_private_key(&private_key_test_another()),
            })
        });

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let res = service
            .locally_hide_signatory(&node_id_test(), &node_id_test_other())
            .await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_filter_locally_hidden_signatories() {
        let (
            mut storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();

        storage
            .expect_get_local_signatory_overrides()
            .returning(|_| {
                Ok(vec![LocalSignatoryOverride {
                    company_id: node_id_test(),
                    node_id: node_id_test(),
                    status: LocalSignatoryOverrideStatus::Hidden,
                }])
            })
            .once();

        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );

        let mut sig = get_valid_activated_signatory(&node_id_test());
        sig.status = CompanySignatoryStatus::Removed {
            ts: test_ts(),
            remover: node_id_test(),
        };
        let mut signatories = vec![sig];

        let res = service
            .filter_out_locally_hidden_signatories(&node_id_test(), &mut signatories)
            .await;
        assert!(res.is_ok());
        assert!(signatories.is_empty());
    }

    #[tokio::test]
    async fn wrong_network_failures() {
        let (
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            nostr_contact_store,
            email_client,
            email_notification_store,
        ) = get_storages();
        let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        let service = get_service(
            storage,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            contact_store,
            nostr_contact_store,
            identity_chain_store,
            company_chain_store,
            transport,
            email_client,
            email_notification_store,
        );
        assert!(service.list_signatories(&mainnet_node_id).await.is_err());
        assert!(
            service
                .get_company_and_keys_by_id(&mainnet_node_id)
                .await
                .is_err()
        );
        assert!(service.get_company_by_id(&mainnet_node_id).await.is_err());
        assert!(
            service
                .edit_company(
                    &mainnet_node_id,
                    None,
                    None,
                    None,
                    None,
                    EditOptionalFieldMode::Ignore,
                    None,
                    EditOptionalFieldMode::Ignore,
                    EditOptionalFieldMode::Ignore,
                    EditOptionalFieldMode::Ignore,
                    EditOptionalFieldMode::Ignore,
                    EditOptionalFieldMode::Ignore,
                    EditOptionalFieldMode::Ignore,
                    test_ts()
                )
                .await
                .is_err()
        );
        assert!(
            service
                .invite_signatory(&mainnet_node_id.clone(), mainnet_node_id.clone(), test_ts())
                .await
                .is_err()
        );
        assert!(
            service
                .remove_signatory(&mainnet_node_id.clone(), mainnet_node_id, test_ts())
                .await
                .is_err()
        );
    }
}
