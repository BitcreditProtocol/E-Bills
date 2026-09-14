use super::Result;
use crate::constants::{
    IDENTITY_DOCUMENT_FILE_FIELD, IDENTITY_PROFILE_PICTURE_FILE_FIELD,
    SAVE_SEED_PHRASE_NOTIFICATION_KEY, SAVE_SEED_PHRASE_NOTIFICATION_REFERENCE_ID,
};
use crate::external::email::EmailClientApi;
use crate::external::file_storage::FileStorageClientApi;
use crate::service::Error;
use crate::service::file_reference_helper::{encrypt_upload_and_track_file, identity_file_context};
use crate::service::file_server_service::{
    configured_blossom_servers, download_file_with_fallback,
};
use crate::service::file_upload_service::UploadFileType;
use crate::service::transport_service::{BcrMetadata, NostrContactData, TransportServiceApi};
use crate::util::validate_node_id_network;
use crate::{get_config, util};
use bcr_ebill_core::application::contact::Contact;
use bcr_ebill_core::protocol::blockchain::bill::ContactType;
use bcr_ebill_core::protocol::{Address, EditOptionalFieldMode, Zip};

use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::application::identity::validation::validate_create_identity;
use bcr_ebill_core::application::identity::{ActiveIdentityState, Identity, IdentityWithAll};
use bcr_ebill_core::application::{ServiceTraitBounds, ValidationError};
use bcr_ebill_core::protocol::Identification;
use bcr_ebill_core::protocol::Name;
use bcr_ebill_core::protocol::Sha256Hash;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::blockchain::Blockchain;
use bcr_ebill_core::protocol::blockchain::identity::{
    IdentityBlock, IdentityBlockPlaintextWrapper, IdentityBlockchain, IdentityCreateBlockData,
    IdentityOpCode, IdentityProofBlockData, IdentityType, IdentityUpdateBlockData,
    IdentityValidateActionData,
};
use bcr_ebill_core::protocol::crypto::{self, BcrKeys, DeriveKeypair};
use bcr_ebill_core::protocol::{City, ProtocolValidationError};
use bcr_ebill_core::protocol::{Country, EmailIdentityProofData, SignedIdentityProof};
use bcr_ebill_core::protocol::{Date, Field};
use bcr_ebill_core::protocol::{Email, blockchain};
use bcr_ebill_core::protocol::{File, OptionalPostalAddress, Validate, event::IdentityChainEvent};
use bcr_ebill_persistence::file_upload::FileUploadStoreApi;
use bcr_ebill_persistence::identity::{IdentityChainStoreApi, IdentityStoreApi};
use bcr_ebill_persistence::notification::EmailNotificationStoreApi;
use bcr_ebill_persistence::{ContactStoreApi, FileReferenceStoreApi};
use bitcoin::base58;
use bitcoin::secp256k1::{PublicKey, SecretKey};
use log::{debug, error};
use std::sync::Arc;
use uuid::Uuid;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait IdentityServiceApi: ServiceTraitBounds {
    /// Updates the identity
    async fn update_identity(
        &self,
        name: Option<Name>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        date_of_birth: EditOptionalFieldMode<Date>,
        country_of_birth: EditOptionalFieldMode<Country>,
        city_of_birth: EditOptionalFieldMode<City>,
        identification_number: EditOptionalFieldMode<Identification>,
        profile_picture_file_upload_id: EditOptionalFieldMode<Uuid>,
        identity_document_file_upload_id: EditOptionalFieldMode<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()>;
    /// Updates the identity email
    async fn update_email(&self, email: &Email, timestamp: Timestamp) -> Result<()>;
    /// Gets the full local identity, including the key pair and node id
    async fn get_full_identity(&self) -> Result<IdentityWithAll>;
    /// Gets the local identity
    async fn get_identity(&self) -> Result<Identity>;
    /// Checks if the identity has been created
    async fn identity_exists(&self) -> bool;
    /// Creates the identity
    async fn create_identity(
        &self,
        t: IdentityType,
        name: Name,
        email: Option<Email>,
        postal_address: OptionalPostalAddress,
        date_of_birth: Option<Date>,
        country_of_birth: Option<Country>,
        city_of_birth: Option<City>,
        identification_number: Option<Identification>,
        profile_picture_file_upload_id: Option<Uuid>,
        identity_document_file_upload_id: Option<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()>;
    /// Deanonymizes an anon identity
    async fn deanonymize_identity(
        &self,
        t: IdentityType,
        name: Name,
        email: Option<Email>,
        postal_address: OptionalPostalAddress,
        date_of_birth: Option<Date>,
        country_of_birth: Option<Country>,
        city_of_birth: Option<City>,
        identification_number: Option<Identification>,
        profile_picture_file_upload_id: Option<Uuid>,
        identity_document_file_upload_id: Option<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()>;
    async fn get_seedphrase(&self) -> Result<String>;
    /// Recovers the private keys in the identity from a seed phrase
    async fn recover_from_seedphrase(&self, seed: &str) -> Result<()>;

    /// opens and decrypts the attached file from the identity
    async fn open_and_decrypt_file(
        &self,
        identity: Identity,
        id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>>;

    /// gets the currently set identity
    async fn get_current_identity(&self) -> Result<ActiveIdentityState>;

    /// sets the active identity to the given personal node id
    async fn set_current_personal_identity(&self, node_id: &NodeId) -> Result<()>;

    /// sets the active identity to the given company node id
    async fn set_current_company_identity(&self, node_id: &NodeId) -> Result<()>;

    /// Returns the local key pair
    async fn get_keys(&self) -> Result<BcrKeys>;

    /// Shares derived keys for private identity contact information. Recipient is the given node id.
    async fn share_contact_details(&self, share_to: &NodeId) -> Result<()>;

    /// Publishes this identity's contact to the nostr profile
    async fn publish_contact(&self, identity: &Identity, keys: &BcrKeys) -> Result<()>;

    /// If dev mode is on, return the full identity chain with decrypted data
    async fn dev_mode_get_full_identity_chain(&self) -> Result<Vec<IdentityBlockPlaintextWrapper>>;

    /// Confirm a new email address
    async fn confirm_email(&self, email: &Email) -> Result<()>;

    /// Verify confirmation of an email address with the sent confirmation code
    async fn verify_email(&self, confirmation_code: &str) -> Result<()>;

    /// Get email confirmations for the identity
    async fn get_email_confirmations(
        &self,
    ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>>;
}

/// The identity service is responsible for managing the local identity
#[derive(Clone)]
pub struct IdentityService {
    store: Arc<dyn IdentityStoreApi>,
    file_upload_store: Arc<dyn FileUploadStoreApi>,
    file_upload_client: Arc<dyn FileStorageClientApi>,
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
    blockchain_store: Arc<dyn IdentityChainStoreApi>,
    block_transport: Arc<dyn TransportServiceApi>,
    email_client: Arc<dyn EmailClientApi>,
    email_notification_store: Arc<dyn EmailNotificationStoreApi>,
    contact_store: Arc<dyn ContactStoreApi>,
}

impl IdentityService {
    pub fn new(
        store: Arc<dyn IdentityStoreApi>,
        file_upload_store: Arc<dyn FileUploadStoreApi>,
        file_upload_client: Arc<dyn FileStorageClientApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        blockchain_store: Arc<dyn IdentityChainStoreApi>,
        block_transport: Arc<dyn TransportServiceApi>,
        email_client: Arc<dyn EmailClientApi>,
        email_notification_store: Arc<dyn EmailNotificationStoreApi>,
        contact_store: Arc<dyn ContactStoreApi>,
    ) -> Self {
        Self {
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            blockchain_store,
            block_transport,
            email_client,
            email_notification_store,
            contact_store,
        }
    }

    async fn process_upload_file(
        &self,
        upload_id: &Option<Uuid>,
        id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        upload_file_type: UploadFileType,
        field: &str,
    ) -> Result<Option<File>> {
        if let Some(upload_id) = upload_id {
            debug!("processing upload file for identity {id}: {upload_id:?}");
            let (file_name, file_bytes) = &self
                .file_upload_store
                .read_temp_upload_file(upload_id)
                .await
                .map_err(|_| {
                    crate::service::Error::Validation(ValidationError::NoFileForFileUploadId)
                })?;
            if !upload_file_type.check_file_size(file_bytes.len()) {
                return Err(crate::service::Error::Validation(
                    ProtocolValidationError::FileIsTooBig(upload_file_type.max_file_size()).into(),
                ));
            }
            let file = self
                .encrypt_and_save_uploaded_file(
                    file_name, file_bytes, id, public_key, signer, field,
                )
                .await?;
            return Ok(Some(file));
        }
        Ok(None)
    }

    async fn encrypt_and_save_uploaded_file(
        &self,
        file_name: &Name,
        file_bytes: &[u8],
        node_id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        field: &str,
    ) -> Result<File> {
        encrypt_upload_and_track_file(
            &self.file_reference_store,
            &self.file_upload_client,
            &self.block_transport,
            &configured_blossom_servers(&get_config().nostr_config),
            file_name,
            file_bytes,
            public_key,
            signer,
            node_id,
            identity_file_context(field),
            None,
            "identity",
        )
        .await
    }

    async fn populate_block(
        &self,
        identity: &Identity,
        block: &IdentityBlock,
        keys: &BcrKeys,
    ) -> Result<()> {
        self.block_transport
            .block_transport()
            .send_identity_chain_events(IdentityChainEvent::new(&identity.node_id, block, keys))
            .await?;
        Ok(())
    }

    async fn on_identity_contact_change(&self, identity: &Identity, keys: &BcrKeys) -> Result<()> {
        debug!("Identity change");
        self.publish_contact(identity, keys).await
    }

    async fn validate_and_add_block(
        &self,
        chain: &mut IdentityBlockchain,
        new_block: IdentityBlock,
    ) -> Result<()> {
        let try_add_block = chain.try_add_block(new_block.clone());
        if try_add_block && chain.is_chain_valid() {
            self.blockchain_store.add_block(&new_block).await?;
            Ok(())
        } else {
            Err(Error::Protocol(blockchain::Error::BlockchainInvalid.into()))
        }
    }

    async fn create_identity_proof_block(
        &self,
        proof: SignedIdentityProof,
        data: EmailIdentityProofData,
        identity: &Identity,
        keys: &BcrKeys,
        chain: &mut IdentityBlockchain,
    ) -> Result<()> {
        let new_block = IdentityBlock::create_block_for_identity_proof(
            chain.get_latest_block(),
            &IdentityProofBlockData { proof, data },
            keys,
            Timestamp::now(),
        )
        .map_err(|e| Error::Protocol(e.into()))?;

        self.validate_and_add_block(chain, new_block.clone())
            .await?;
        self.populate_block(identity, &new_block, keys).await?;

        Ok(())
    }

    /// If mandatory email confirmations are not disabled, check that there is a confirmed email and the
    /// given email is a confirmed one for the identity
    async fn check_confirmed_email(
        &self,
        email: &Option<Email>,
        t: &IdentityType,
        node_id: &NodeId,
    ) -> Result<Option<(SignedIdentityProof, EmailIdentityProofData)>> {
        match t {
            IdentityType::Ident => {
                // Email has to be checked before
                let Some(em) = email else {
                    return Err(ProtocolValidationError::FieldEmpty(Field::Email).into());
                };

                if get_config().dev_mode_config.mandatory_email_confirmations {
                    // Make sure there is a confirmed email
                    let email_confirmations = self.store.get_email_confirmations().await?;
                    if email_confirmations.is_empty() {
                        return Err(Error::Validation(
                            ValidationError::NoConfirmedEmailForIdentIdentity,
                        ));
                    }

                    // Given email has to be a confirmed email
                    for ec in email_confirmations.iter() {
                        if &ec.1.email == em {
                            return Ok(Some((ec.0.to_owned(), ec.1.to_owned())));
                        }
                    }

                    // No email found - fail
                    Err(Error::Validation(
                        ValidationError::NoConfirmedEmailForIdentIdentity,
                    ))
                } else {
                    // if mandatory email confirmations are disabled, create self-signed email confirmation
                    let keys = self.get_keys().await?;

                    let self_signed_identity = EmailIdentityProofData {
                        node_id: node_id.to_owned(),
                        company_node_id: None,
                        email: em.to_owned(),
                        created_at: Timestamp::now(),
                    };
                    let proof = self_signed_identity.sign(node_id, &keys.get_private_key())?;
                    self.store
                        .set_email_confirmation(&proof, &self_signed_identity)
                        .await?;

                    Ok(Some((proof, self_signed_identity)))
                }
            }
            IdentityType::Anon => Ok(None),
        }
    }
}

/// Derives a child key, encrypts the contact data with it and returns the bcr metadata
fn get_bcr_data(
    identity: &Identity,
    keys: &BcrKeys,
    mint_url: Option<url::Url>,
) -> Result<BcrMetadata> {
    let derived_keys = keys.derive_identity_keypair()?;
    let mut contact = identity.as_contact(None);
    contact.mint_url = mint_url;
    debug!("Publishing identity contact data: {contact:?}");
    let payload = serde_json::to_string(&contact)?;
    let encrypted = base58::encode(&crypto::encrypt_ecies(
        payload.as_bytes(),
        &derived_keys.public_key(),
    )?);
    Ok(BcrMetadata {
        contact_data: encrypted,
    })
}

impl ServiceTraitBounds for IdentityService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl IdentityServiceApi for IdentityService {
    async fn get_full_identity(&self) -> Result<IdentityWithAll> {
        let identity = self.store.get_full().await?;
        Ok(identity)
    }

    async fn update_identity(
        &self,
        name: Option<Name>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        date_of_birth: EditOptionalFieldMode<Date>,
        country_of_birth: EditOptionalFieldMode<Country>,
        city_of_birth: EditOptionalFieldMode<City>,
        identification_number: EditOptionalFieldMode<Identification>,
        profile_picture_file_upload_id: EditOptionalFieldMode<Uuid>,
        identity_document_file_upload_id: EditOptionalFieldMode<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!("updating identity");
        let mut identity = self.store.get().await?;
        let mut changed = false;

        let mut profile_picture_file = None;
        let mut identity_document_file = None;

        let keys = self.store.get_key_pair().await?;

        if let Some(ref name_to_set) = name
            && &identity.name != name_to_set
        {
            identity.name = name_to_set.to_owned();
            changed = true;
        }

        // for anonymous identity, we only consider name
        if identity.t == IdentityType::Ident {
            if let Some(ref country_to_set) = country
                && identity.postal_address.country.as_ref() != Some(country_to_set)
            {
                identity.postal_address.country = Some(country_to_set.to_owned());
                changed = true;
            }

            if let Some(ref city_to_set) = city
                && identity.postal_address.city.as_ref() != Some(city_to_set)
            {
                identity.postal_address.city = Some(city_to_set.to_owned());
                changed = true;
            }

            util::handle_optional_field(&mut identity.postal_address.zip, &zip, &mut changed);

            if let Some(ref address_to_set) = address
                && identity.postal_address.address.as_ref() != Some(address_to_set)
            {
                identity.postal_address.address = Some(address_to_set.to_owned());
                changed = true;
            }

            util::handle_optional_field(&mut identity.date_of_birth, &date_of_birth, &mut changed);

            util::handle_optional_field(
                &mut identity.country_of_birth,
                &country_of_birth,
                &mut changed,
            );

            util::handle_optional_field(&mut identity.city_of_birth, &city_of_birth, &mut changed);

            util::handle_optional_field(
                &mut identity.identification_number,
                &identification_number,
                &mut changed,
            );

            profile_picture_file = match profile_picture_file_upload_id {
                EditOptionalFieldMode::Set(profile_picture_file_upload_id) => {
                    let profile_picture_file = self
                        .process_upload_file(
                            &Some(profile_picture_file_upload_id),
                            &identity.node_id,
                            &keys.pub_key(),
                            &keys,
                            UploadFileType::Picture,
                            IDENTITY_PROFILE_PICTURE_FILE_FIELD,
                        )
                        .await?;
                    if profile_picture_file.is_some() {
                        identity.profile_picture_file = profile_picture_file.clone();
                        changed = true;
                    }
                    profile_picture_file
                }
                EditOptionalFieldMode::Unset => {
                    identity.profile_picture_file = None;
                    changed = true;
                    None
                }
                EditOptionalFieldMode::Ignore => None,
            };

            identity_document_file = match identity_document_file_upload_id {
                EditOptionalFieldMode::Set(identity_document_file_upload_id) => {
                    let identity_document_file = self
                        .process_upload_file(
                            &Some(identity_document_file_upload_id),
                            &identity.node_id,
                            &keys.pub_key(),
                            &keys,
                            UploadFileType::Document,
                            IDENTITY_DOCUMENT_FILE_FIELD,
                        )
                        .await?;
                    if identity_document_file.is_some() {
                        identity.identity_document_file = identity_document_file.clone();
                        changed = true;
                    }
                    identity_document_file
                }
                EditOptionalFieldMode::Unset => {
                    identity.identity_document_file = None;
                    changed = true;
                    None
                }
                EditOptionalFieldMode::Ignore => None,
            };

            if !changed {
                log::warn!("Called Identity Change without any changes - returning");
                return Ok(());
            }
        }

        let mut identity_chain = self.blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: identity.node_id.clone(),
            op: IdentityOpCode::Update,
            keys: keys.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_block = identity_chain.get_latest_block();
        let block_data = IdentityUpdateBlockData {
            t: None,
            name,
            email: None,
            country,
            city,
            zip,
            address,
            date_of_birth,
            country_of_birth,
            city_of_birth,
            identification_number,
            profile_picture_file: profile_picture_file_upload_id.swap_value(profile_picture_file),
            identity_document_file: identity_document_file_upload_id
                .swap_value(identity_document_file),
        };
        block_data.validate()?;

        let new_block =
            IdentityBlock::create_block_for_update(previous_block, &block_data, &keys, timestamp)
                .map_err(|e| Error::Protocol(e.into()))?;
        self.validate_and_add_block(&mut identity_chain, new_block.clone())
            .await?;

        self.store.save(&identity).await?;
        self.populate_block(&identity, &new_block, &keys).await?;
        self.on_identity_contact_change(&identity, &keys).await?;
        debug!("updated identity");
        Ok(())
    }

    async fn update_email(&self, email: &Email, timestamp: Timestamp) -> Result<()> {
        debug!("updating identity email");
        let mut identity = self.store.get().await?;
        let keys = self.store.get_key_pair().await?;

        if identity.t == IdentityType::Ident {
            if identity.email.as_ref() != Some(email) {
                identity.email = Some(email.to_owned());
            } else {
                // return early, if email didn't change
                return Ok(());
            }
        } else {
            // can't update email for anon identity
            return Err(Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }

        // check if email is confirmed
        let email_confirmation = self
            .check_confirmed_email(&Some(email.to_owned()), &identity.t, &identity.node_id)
            .await?;

        let mut identity_chain = self.blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: identity.node_id.clone(),
            op: IdentityOpCode::Update,
            keys: keys.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_block = identity_chain.get_latest_block();
        let block_data = IdentityUpdateBlockData {
            t: None,
            name: None,
            email: Some(email.to_owned()),
            country: None,
            city: None,
            zip: EditOptionalFieldMode::Ignore,
            address: None,
            date_of_birth: EditOptionalFieldMode::Ignore,
            country_of_birth: EditOptionalFieldMode::Ignore,
            city_of_birth: EditOptionalFieldMode::Ignore,
            identification_number: EditOptionalFieldMode::Ignore,
            profile_picture_file: EditOptionalFieldMode::Ignore,
            identity_document_file: EditOptionalFieldMode::Ignore,
        };
        block_data.validate()?;
        let new_block =
            IdentityBlock::create_block_for_update(previous_block, &block_data, &keys, timestamp)
                .map_err(|e| Error::Protocol(e.into()))?;
        self.validate_and_add_block(&mut identity_chain, new_block.clone())
            .await?;

        self.store.save(&identity).await?;
        self.populate_block(&identity, &new_block, &keys).await?;
        self.on_identity_contact_change(&identity, &keys).await?;

        // Create and populate identity proof block
        if let Some((proof, data)) = email_confirmation {
            IdentityValidateActionData {
                blockchain: identity_chain.clone(),
                id: identity.node_id.clone(),
                op: IdentityOpCode::IdentityProof,
                keys: keys.clone(),
                identity_proof_data: Some((proof.clone(), data.clone())),
            }
            .validate()?;
            self.create_identity_proof_block(proof, data, &identity, &keys, &mut identity_chain)
                .await?;
        }

        debug!("updated identity email");
        Ok(())
    }

    async fn get_identity(&self) -> Result<Identity> {
        let identity = self.store.get().await?;
        Ok(identity)
    }

    async fn identity_exists(&self) -> bool {
        self.store.exists().await
    }

    async fn create_identity(
        &self,
        t: IdentityType,
        name: Name,
        email: Option<Email>,
        postal_address: OptionalPostalAddress,
        date_of_birth: Option<Date>,
        country_of_birth: Option<Country>,
        city_of_birth: Option<City>,
        identification_number: Option<Identification>,
        profile_picture_file_upload_id: Option<Uuid>,
        identity_document_file_upload_id: Option<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!("creating identity");
        let keys = self.store.get_or_create_key_pair().await?;
        let node_id = NodeId::new(keys.pub_key(), get_config().bitcoin_network());
        validate_create_identity(t.clone(), &email, &postal_address)?;
        let email_confirmation = self.check_confirmed_email(&email, &t, &node_id).await?;

        let identity = match t {
            IdentityType::Ident => {
                let profile_picture_file = self
                    .process_upload_file(
                        &profile_picture_file_upload_id,
                        &node_id,
                        &keys.pub_key(),
                        &keys,
                        UploadFileType::Picture,
                        IDENTITY_PROFILE_PICTURE_FILE_FIELD,
                    )
                    .await?;

                let identity_document_file = self
                    .process_upload_file(
                        &identity_document_file_upload_id,
                        &node_id,
                        &keys.pub_key(),
                        &keys,
                        UploadFileType::Document,
                        IDENTITY_DOCUMENT_FILE_FIELD,
                    )
                    .await?;

                Identity {
                    t: t.clone(),
                    node_id: node_id.clone(),
                    name,
                    email,
                    postal_address,
                    date_of_birth,
                    country_of_birth,
                    city_of_birth,
                    identification_number,
                    profile_picture_file,
                    identity_document_file,
                    nostr_relays: get_config().nostr_config.relays.to_owned(),
                }
            }
            IdentityType::Anon => Identity {
                t: t.clone(),
                node_id: node_id.clone(),
                name,
                email: None,
                postal_address: OptionalPostalAddress::empty(),
                date_of_birth: None,
                country_of_birth: None,
                city_of_birth: None,
                identification_number: None,
                profile_picture_file: None,
                identity_document_file: None,
                nostr_relays: get_config().nostr_config.relays.to_owned(),
            },
        };

        // create new identity chain and persist it
        let block_data: IdentityCreateBlockData = identity.clone().into();
        block_data.validate()?;
        let mut identity_chain = IdentityBlockchain::new(&block_data, &keys, timestamp)
            .map_err(|e| Error::Protocol(e.into()))?;
        let first_block = identity_chain.get_first_block();
        self.blockchain_store.add_block(first_block).await?;

        // persist the identity in the DB
        self.store.save(&identity).await?;
        self.populate_block(&identity, first_block, &keys).await?;
        self.on_identity_contact_change(&identity, &keys).await?;

        // add to contacts without files if it's not there already, but don't fail
        if let Ok(None) = self.contact_store.get(&node_id).await
            && let Err(e) = self
                .contact_store
                .insert(
                    &node_id,
                    Contact {
                        t: if matches!(t, IdentityType::Ident) {
                            ContactType::Person
                        } else {
                            ContactType::Anon
                        },
                        node_id: identity.node_id.to_owned(),
                        name: identity.name.clone(),
                        email: identity.email.clone(),
                        postal_address: identity.postal_address.to_full_postal_address(),
                        date_of_birth_or_registration: identity.date_of_birth.clone(),
                        country_of_birth_or_registration: identity.country_of_birth.clone(),
                        city_of_birth_or_registration: identity.city_of_birth.clone(),
                        identification_number: identity.identification_number.clone(),
                        avatar_file: None,
                        proof_document_file: None,
                        nostr_relays: get_config().nostr_config.relays.clone(),
                        is_logical: false,
                        mint_url: Some(get_config().mint_config.default_mint_url.clone()),
                    },
                )
                .await
        {
            error!("Could not create contact for own identity {node_id}: {e}");
        }

        if let Err(e) = self
            .block_transport
            .notification_transport()
            .create_general_notification(
                &node_id,
                SAVE_SEED_PHRASE_NOTIFICATION_KEY,
                Some(SAVE_SEED_PHRASE_NOTIFICATION_REFERENCE_ID.to_string()),
                bcr_ebill_core::application::notification::NotificationLevel::ActionRequired,
            )
            .await
        {
            error!("Failed to create save seed phrase notification: {e}");
        }

        // Create and populate identity proof block
        if let Some((proof, data)) = email_confirmation {
            IdentityValidateActionData {
                blockchain: identity_chain.clone(),
                id: identity.node_id.clone(),
                op: IdentityOpCode::IdentityProof,
                keys: keys.clone(),
                identity_proof_data: Some((proof.clone(), data.clone())),
            }
            .validate()?;
            self.create_identity_proof_block(proof, data, &identity, &keys, &mut identity_chain)
                .await?;
        }

        debug!("created identity");
        Ok(())
    }

    async fn deanonymize_identity(
        &self,
        t: IdentityType,
        name: Name,
        email: Option<Email>,
        postal_address: OptionalPostalAddress,
        date_of_birth: Option<Date>,
        country_of_birth: Option<Country>,
        city_of_birth: Option<City>,
        identification_number: Option<Identification>,
        profile_picture_file_upload_id: Option<Uuid>,
        identity_document_file_upload_id: Option<Uuid>,
        timestamp: Timestamp,
    ) -> Result<()> {
        debug!("deanonymizing identity");
        let existing_identity = self.store.get().await?;
        let keys = self.store.get_key_pair().await?;
        // can't de-anonymize to an anonymous identity
        if t == IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::IdentityCantBeAnon.into(),
            ));
        }

        // if the existing identity is not anon, the action is not valid
        if existing_identity.t != IdentityType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::InvalidIdentityType.into(),
            ));
        }

        validate_create_identity(t.clone(), &email, &postal_address)?;

        let email_confirmation = self
            .check_confirmed_email(&email, &t, &existing_identity.node_id)
            .await?;

        let profile_picture_file = self
            .process_upload_file(
                &profile_picture_file_upload_id,
                &existing_identity.node_id,
                &keys.pub_key(),
                &keys,
                UploadFileType::Picture,
                IDENTITY_PROFILE_PICTURE_FILE_FIELD,
            )
            .await?;

        let identity_document_file = self
            .process_upload_file(
                &identity_document_file_upload_id,
                &existing_identity.node_id,
                &keys.pub_key(),
                &keys,
                UploadFileType::Document,
                IDENTITY_DOCUMENT_FILE_FIELD,
            )
            .await?;

        let identity = Identity {
            t: t.clone(),
            node_id: existing_identity.node_id.clone(),
            name: name.clone(),
            email: email.clone(),
            postal_address: postal_address.clone(),
            date_of_birth: date_of_birth.clone(),
            country_of_birth: country_of_birth.clone(),
            city_of_birth: city_of_birth.clone(),
            identification_number: identification_number.clone(),
            profile_picture_file: profile_picture_file.clone(),
            identity_document_file: identity_document_file.clone(),
            nostr_relays: get_config().nostr_config.relays.to_owned(),
        };

        let mut identity_chain = self.blockchain_store.get_chain().await?;
        IdentityValidateActionData {
            blockchain: identity_chain.clone(),
            id: identity.node_id.clone(),
            op: IdentityOpCode::Update,
            keys: keys.clone(),
            identity_proof_data: None,
        }
        .validate()?;
        let previous_block = identity_chain.get_latest_block();
        let block_data = IdentityUpdateBlockData {
            t: Some(t.clone()),
            name: Some(name),
            email,
            country: postal_address.country,
            city: postal_address.city,
            zip: EditOptionalFieldMode::from_option_no_unset(postal_address.zip),
            address: postal_address.address,
            date_of_birth: EditOptionalFieldMode::from_option_no_unset(date_of_birth),
            country_of_birth: EditOptionalFieldMode::from_option_no_unset(country_of_birth),
            city_of_birth: EditOptionalFieldMode::from_option_no_unset(city_of_birth),
            identification_number: EditOptionalFieldMode::from_option_no_unset(
                identification_number,
            ),
            profile_picture_file: EditOptionalFieldMode::from_option_no_unset(profile_picture_file),
            identity_document_file: EditOptionalFieldMode::from_option_no_unset(
                identity_document_file,
            ),
        };
        block_data.validate()?;
        let new_block =
            IdentityBlock::create_block_for_update(previous_block, &block_data, &keys, timestamp)
                .map_err(|e| Error::Protocol(e.into()))?;
        self.validate_and_add_block(&mut identity_chain, new_block.clone())
            .await?;
        self.store.save(&identity).await?;
        self.populate_block(&identity, &new_block, &keys).await?;
        self.on_identity_contact_change(&identity, &keys).await?;

        // Create and populate identity proof block
        if let Some((proof, data)) = email_confirmation {
            self.create_identity_proof_block(proof, data, &identity, &keys, &mut identity_chain)
                .await?;
        }

        debug!("deanonymized identity");
        Ok(())
    }

    /// Recovers keys from a seed phrase and stores them into the identity
    async fn recover_from_seedphrase(&self, seed: &str) -> Result<()> {
        let key_pair = BcrKeys::from_seedphrase(seed)?;
        self.store.save_key_pair(&key_pair, seed).await?;
        Ok(())
    }

    async fn get_seedphrase(&self) -> Result<String> {
        let res = self.store.get_seedphrase().await?;
        Ok(res)
    }

    async fn open_and_decrypt_file(
        &self,
        identity: Identity,
        id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>> {
        validate_node_id_network(id)?;
        debug!("getting file {file_name} for identity with id: {id}");
        let mut file = None;

        if let Some(profile_picture_file) = identity.profile_picture_file
            && &profile_picture_file.name == file_name
        {
            file = Some(profile_picture_file);
        }

        if let Some(identity_document_file) = identity.identity_document_file
            && &identity_document_file.name == file_name
        {
            file = Some(identity_document_file);
        }

        if let Some(file) = file {
            let file_bytes = download_file_with_fallback(
                self.file_upload_client.as_ref(),
                Some(&self.file_reference_store),
                Some(&self.block_transport),
                &configured_blossom_servers(&get_config().nostr_config),
                &file.hash,
                &file.nostr_hash,
            )
            .await?;
            let decrypted = crypto::decrypt_ecies(&file_bytes, private_key)?;
            let file_hash = Sha256Hash::from_bytes(&decrypted);
            if file_hash != file.hash {
                error!("Hash for identity file {file_name} did not match uploaded file");
                return Err(super::Error::NotFound);
            }
            Ok(decrypted.to_vec())
        } else {
            Err(super::Error::NotFound)
        }
    }

    async fn get_current_identity(&self) -> Result<ActiveIdentityState> {
        let active_identity = self.store.get_current_identity().await?;
        Ok(active_identity)
    }

    async fn set_current_personal_identity(&self, node_id: &NodeId) -> Result<()> {
        validate_node_id_network(node_id)?;
        debug!("setting current identity to personal identity: {node_id}");
        self.store
            .set_current_identity(&ActiveIdentityState {
                personal: node_id.to_owned(),
                company: None,
            })
            .await?;
        Ok(())
    }

    async fn set_current_company_identity(&self, node_id: &NodeId) -> Result<()> {
        validate_node_id_network(node_id)?;
        debug!("setting current identity to company identity: {node_id}");
        let active_identity = self.store.get_current_identity().await?;
        self.store
            .set_current_identity(&ActiveIdentityState {
                personal: active_identity.personal,
                company: Some(node_id.to_owned()),
            })
            .await?;
        Ok(())
    }

    async fn get_keys(&self) -> Result<BcrKeys> {
        Ok(self.store.get_key_pair().await?)
    }

    async fn share_contact_details(&self, share_to: &NodeId) -> Result<()> {
        let identity = self.get_full_identity().await?;
        let derived_keys = identity.key_pair.derive_identity_keypair()?;
        let keys = BcrKeys::from_private_key(&derived_keys.secret_key());
        self.block_transport
            .contact_transport()
            .share_contact_details_keys(share_to, &identity.identity.node_id, &keys, None)
            .await?;
        Ok(())
    }

    async fn dev_mode_get_full_identity_chain(&self) -> Result<Vec<IdentityBlockPlaintextWrapper>> {
        // if dev mode is off - we return an error
        if !get_config().dev_mode_config.on {
            error!("Called dev mode operation with dev mode disabled - please enable!");
            return Err(Error::Validation(ValidationError::InvalidOperation));
        }

        // if there is identity yet, we return an error
        if !self.identity_exists().await {
            return Err(Error::NotFound);
        }

        let chain = self.blockchain_store.get_chain().await?;
        let keys = self.store.get_key_pair().await?;

        let plaintext_chain = chain
            .get_chain_with_plaintext_block_data(&keys)
            .map_err(|e| Error::Protocol(e.into()))?;

        Ok(plaintext_chain)
    }

    async fn publish_contact(&self, identity: &Identity, keys: &BcrKeys) -> Result<()> {
        debug!("Publishing our identity contact to nostr profile");
        let mint_url = Some(get_config().mint_config.default_mint_url.clone());
        let bcr_data = get_bcr_data(identity, keys, mint_url)?;
        let contact_data = NostrContactData::new(
            &identity.name,
            identity.nostr_relays.clone(),
            configured_blossom_servers(&get_config().nostr_config),
            bcr_data,
        );
        self.block_transport
            .contact_transport()
            .publish_contact(&identity.node_id, &contact_data)
            .await?;
        self.block_transport
            .contact_transport()
            .ensure_nostr_contact(&identity.node_id)
            .await;
        Ok(())
    }

    async fn confirm_email(&self, email: &Email) -> Result<()> {
        let keys = self.store.get_or_create_key_pair().await?;
        let node_id = NodeId::new(keys.pub_key(), get_config().bitcoin_network());

        // use default mint URL for now, until we support multiple mints
        let mint_url = get_config().mint_config.default_mint_url.to_owned();

        self.email_client
            .register(&mint_url, &node_id, &None, email, &keys.get_private_key())
            .await?;

        Ok(())
    }

    async fn verify_email(&self, confirmation_code: &str) -> Result<()> {
        let keys = self.store.get_or_create_key_pair().await?;
        let node_id = NodeId::new(keys.pub_key(), get_config().bitcoin_network());

        // use default mint URL for now, until we support multiple mints
        let mint_url = get_config().mint_config.default_mint_url.to_owned();
        let mint_node_id = get_config().mint_config.default_mint_node_id.to_owned();

        let (signed_proof, signed_email_identity_data) = self
            .email_client
            .confirm(
                &mint_url,
                &mint_node_id,
                &node_id,
                &None,
                confirmation_code,
                &keys.get_private_key(),
            )
            .await?;

        self.store
            .set_email_confirmation(&signed_proof, &signed_email_identity_data)
            .await?;

        let email_preferences_link = self
            .email_client
            .get_email_preferences_link(&mint_url, &node_id, &None, &keys.get_private_key())
            .await?;
        self.email_notification_store
            .add_email_preferences_link_for_node_id(&email_preferences_link, &node_id)
            .await?;

        Ok(())
    }

    async fn get_email_confirmations(
        &self,
    ) -> Result<Vec<(SignedIdentityProof, EmailIdentityProofData)>> {
        let email_confirmations = self.store.get_email_confirmations().await?;
        Ok(email_confirmations)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::{
        external::{email::MockEmailClientApi, file_storage::MockFileStorageClientApi},
        service::transport_service::MockTransportServiceApi,
        tests::tests::{
            MockContactStoreApiMock, MockEmailNotificationStoreApiMock,
            MockFileReferenceStoreApiMock, MockFileUploadStoreApiMock,
            MockIdentityChainStoreApiMock, MockIdentityStoreApiMock, empty_identity,
            empty_optional_address, filled_optional_address, init_test_cfg, node_id_test,
            private_key_test, signed_identity_proof_test, test_ts,
        },
    };
    use mockall::predicate::eq;

    fn get_service(mock_storage: MockIdentityStoreApiMock) -> IdentityService {
        IdentityService::new(
            Arc::new(mock_storage),
            Arc::new(MockFileUploadStoreApiMock::new()),
            Arc::new(MockFileStorageClientApi::new()),
            Arc::new(MockFileReferenceStoreApiMock::new()),
            Arc::new(MockIdentityChainStoreApiMock::new()),
            Arc::new(MockTransportServiceApi::new()),
            Arc::new(MockEmailClientApi::new()),
            Arc::new(MockEmailNotificationStoreApiMock::new()),
            Arc::new(MockContactStoreApiMock::new()),
        )
    }

    fn get_service_with_chain_storage(
        mock_storage: MockIdentityStoreApiMock,
        mock_chain_storage: MockIdentityChainStoreApiMock,
        transport: MockTransportServiceApi,
    ) -> IdentityService {
        let mut contact_store = MockContactStoreApiMock::new();
        contact_store.expect_get().returning(|_| Ok(None));
        contact_store.expect_insert().returning(|_, _| Ok(()));

        IdentityService::new(
            Arc::new(mock_storage),
            Arc::new(MockFileUploadStoreApiMock::new()),
            Arc::new(MockFileStorageClientApi::new()),
            Arc::new(MockFileReferenceStoreApiMock::new()),
            Arc::new(mock_chain_storage),
            Arc::new(transport),
            Arc::new(MockEmailClientApi::new()),
            Arc::new(MockEmailNotificationStoreApiMock::new()),
            Arc::new(contact_store),
        )
    }

    fn get_service_with_email_client_and_email_notif_store(
        mock_storage: MockIdentityStoreApiMock,
        mock_email_client: MockEmailClientApi,
        mock_notif_store: MockEmailNotificationStoreApiMock,
    ) -> IdentityService {
        IdentityService::new(
            Arc::new(mock_storage),
            Arc::new(MockFileUploadStoreApiMock::new()),
            Arc::new(MockFileStorageClientApi::new()),
            Arc::new(MockFileReferenceStoreApiMock::new()),
            Arc::new(MockIdentityChainStoreApiMock::new()),
            Arc::new(MockTransportServiceApi::new()),
            Arc::new(mock_email_client),
            Arc::new(mock_notif_store),
            Arc::new(MockContactStoreApiMock::new()),
        )
    }

    #[tokio::test]
    async fn create_identity_baseline() {
        init_test_cfg();
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage
            .expect_get_email_confirmations()
            .returning(|| Ok(vec![signed_identity_proof_test()]));
        storage.expect_save().returning(move |_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage.expect_add_block().returning(|_| Ok(()));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .times(2); // create and identity proof
        });
        transport.expect_on_contact_transport(|t| {
            // publishes contact info to nostr
            t.expect_publish_contact().returning(|_, _| Ok(())).once();
            t.expect_ensure_nostr_contact().returning(|_| ()).once();
        });
        transport.expect_on_notification_transport(|t| {
            t.expect_create_general_notification()
                .returning(|_, _, _, _| Ok(()))
                .once();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .create_identity(
                IdentityType::Ident,
                Name::new("name").unwrap(),
                Some(Email::new("test@example.com").unwrap()),
                filled_optional_address(),
                None,
                None,
                None,
                None,
                None,
                None,
                test_ts(),
            )
            .await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn create_anon_identity_baseline() {
        init_test_cfg();
        let mut storage = MockIdentityStoreApiMock::new();

        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_save().returning(move |_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage.expect_add_block().returning(|_| Ok(()));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        transport.expect_on_contact_transport(|t| {
            t.expect_publish_contact().returning(|_, _| Ok(())).once();
            t.expect_ensure_nostr_contact().returning(|_| ()).once();
        });
        transport.expect_on_notification_transport(|t| {
            t.expect_create_general_notification()
                .returning(|_, _, _, _| Ok(()))
                .once();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .create_identity(
                IdentityType::Anon,
                Name::new("name").unwrap(),
                Some(Email::new("test@example.com").unwrap()),
                empty_optional_address(),
                None,
                None,
                None,
                None,
                None,
                None,
                test_ts(),
            )
            .await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn deanonymize_identity_baseline() {
        init_test_cfg();
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_save().returning(move |_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_get().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = node_id_test();
            identity.t = IdentityType::Anon;
            Ok(identity)
        });
        storage
            .expect_get_email_confirmations()
            .returning(|| Ok(vec![signed_identity_proof_test()]));
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage.expect_add_block().returning(|_| Ok(()));
        chain_storage
            .expect_get_chain()
            .returning(|| Ok(get_genesis_chain(None)));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .times(2); // update and identity proof
        });

        // publishes contact info to nostr
        transport.expect_on_contact_transport(|t| {
            t.expect_publish_contact().returning(|_, _| Ok(())).once();
            t.expect_ensure_nostr_contact().returning(|_| ()).once();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .deanonymize_identity(
                IdentityType::Ident,
                Name::new("name").unwrap(),
                Some(Email::new("test@example.com").unwrap()),
                filled_optional_address(),
                None,
                None,
                None,
                None,
                None,
                None,
                test_ts(),
            )
            .await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn deanonymize_identity_fails_with_anon() {
        init_test_cfg();
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_save().returning(move |_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_get().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = node_id_test();
            identity.t = IdentityType::Anon;
            Ok(identity)
        });
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage.expect_add_block().returning(|_| Ok(()));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events().never();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .deanonymize_identity(
                IdentityType::Anon,
                Name::new("name").unwrap(),
                Some(Email::new("test@example.com").unwrap()),
                empty_optional_address(),
                None,
                None,
                None,
                None,
                None,
                None,
                test_ts(),
            )
            .await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            crate::service::Error::Validation(ValidationError::Protocol(
                ProtocolValidationError::IdentityCantBeAnon
            ))
        ));
    }

    #[tokio::test]
    async fn deanonymize_identity_fails_for_non_anon() {
        init_test_cfg();
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_save().returning(move |_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_get().returning(move || {
            let mut identity = empty_identity();
            identity.node_id = node_id_test();
            Ok(identity)
        });
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage.expect_add_block().returning(|_| Ok(()));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events().never();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .deanonymize_identity(
                IdentityType::Ident,
                Name::new("name").unwrap(),
                Some(Email::new("test@example.com").unwrap()),
                empty_optional_address(),
                None,
                None,
                None,
                None,
                None,
                None,
                test_ts(),
            )
            .await;

        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            crate::service::Error::Validation(ValidationError::Protocol(
                ProtocolValidationError::InvalidIdentityType
            ))
        ));
    }

    #[tokio::test]
    async fn update_identity_calls_storage() {
        let keys = BcrKeys::from_private_key(&private_key_test());
        let mut storage = MockIdentityStoreApiMock::new();
        storage.expect_save().returning(|_| Ok(()));
        storage
            .expect_get_key_pair()
            .returning(move || Ok(keys.clone()));
        storage.expect_get().returning(move || {
            let identity = empty_identity();
            Ok(identity)
        });
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage
            .expect_get_chain()
            .returning(|| Ok(get_genesis_chain(None)));
        chain_storage.expect_add_block().returning(|_| Ok(()));
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events()
                .returning(|_| Ok(()))
                .once();
        });
        transport.expect_on_contact_transport(|t| {
            // publishes contact info to nostr
            t.expect_publish_contact().returning(|_, _| Ok(())).once();
            t.expect_ensure_nostr_contact().returning(|_| ()).once();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .update_identity(
                Some(Name::new("new_name").unwrap()),
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

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn update_identity_returns_if_no_changes_were_made() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage.expect_save().returning(|_| Ok(()));
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_get().returning(move || {
            let mut identity = empty_identity();
            identity.name = Name::new("name").unwrap();
            Ok(identity)
        });

        let service = get_service(storage);
        let res = service
            .update_identity(
                Some(Name::new("name").unwrap()),
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

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn update_identity_propagates_errors() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::from_private_key(&private_key_test())));
        storage.expect_get().returning(move || {
            let identity = empty_identity();
            Ok(identity)
        });
        storage
            .expect_save()
            .returning(|_| Err(bcr_ebill_persistence::Error::EncodingError))
            .times(1);
        let mut chain_storage = MockIdentityChainStoreApiMock::new();
        chain_storage
            .expect_get_chain()
            .returning(|| Ok(get_genesis_chain(None)));
        chain_storage
            .expect_add_block()
            .returning(|_| Ok(()))
            .once();
        let mut transport = MockTransportServiceApi::new();
        transport.expect_on_block_transport(|t| {
            t.expect_send_identity_chain_events().never();
        });

        let service = get_service_with_chain_storage(storage, chain_storage, transport);
        let res = service
            .update_identity(
                Some(Name::new("new_name").unwrap()),
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
    async fn identity_exists_calls_storage() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage.expect_exists().returning(|| true);

        let service = get_service(storage);
        let res = service.identity_exists().await;

        assert!(res);
    }

    #[tokio::test]
    async fn get_identity_calls_storage() {
        let identity = empty_identity();
        let mut storage = MockIdentityStoreApiMock::new();
        storage.expect_get().returning(move || Ok(empty_identity()));

        let service = get_service(storage);
        let res = service.get_identity().await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), identity);
    }

    #[tokio::test]
    async fn get_identity_propagates_errors() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get()
            .returning(|| Err(bcr_ebill_persistence::Error::EncodingError));

        let service = get_service(storage);
        let res = service.get_identity().await;

        assert!(res.is_err());
    }

    #[tokio::test]
    async fn get_full_identity_calls_storage() {
        let identity = IdentityWithAll {
            identity: empty_identity(),
            key_pair: BcrKeys::new(),
        };
        let arced = Arc::new(identity.clone());
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_full()
            .returning(move || Ok((*arced.clone()).clone()));

        let service = get_service(storage);
        let res = service.get_full_identity().await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap().identity, identity.identity);
    }

    #[tokio::test]
    async fn get_full_identity_propagates_errors() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_full()
            .returning(|| Err(bcr_ebill_persistence::Error::EncodingError));

        let service = get_service(storage);
        let res = service.get_full_identity().await;

        assert!(res.is_err());
    }

    #[tokio::test]
    async fn recover_from_seedphrase_stores_key_and_seed() {
        let seed = "forward paper connect economy twelve debate cart isolate accident creek bind predict captain rifle glory cradle hip whisper wealth save buddy place develop dolphin";
        let expected_key = BcrKeys::from_private_key(
            &SecretKey::from_str(
                "f31e0373f6fa9f4835d49a278cd48f47ea115af7480edf435275a3c2dbb1f982",
            )
            .unwrap(),
        );
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_save_key_pair()
            .with(eq(expected_key), eq(seed))
            .returning(|_, _| Ok(()));
        let service = get_service(storage);
        service
            .recover_from_seedphrase(seed)
            .await
            .expect("could not recover from seedphrase")
    }

    #[tokio::test]
    async fn set_current_personal_identity_fails_for_different_network_id() {
        let service = get_service(MockIdentityStoreApiMock::new());
        let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        assert!(
            service
                .set_current_personal_identity(&mainnet_node_id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn set_current_company_identity_fails_for_different_network_id() {
        let service = get_service(MockIdentityStoreApiMock::new());
        let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        assert!(
            service
                .set_current_company_identity(&mainnet_node_id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_verify_email() {
        init_test_cfg();
        let keys = BcrKeys::new();
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(move || Ok(keys.clone()));
        storage
            .expect_set_email_confirmation()
            .returning(|_, _| Ok(()));
        let mut email_client = MockEmailClientApi::new();
        email_client
            .expect_get_email_preferences_link()
            .returning(|_, _, _, _| Ok(url::Url::parse("https://bit.cr").unwrap()));
        email_client
            .expect_confirm()
            .returning(|_, _, _, _, _, _| Ok(signed_identity_proof_test()));
        let mut email_notif_storage = MockEmailNotificationStoreApiMock::new();
        email_notif_storage
            .expect_add_email_preferences_link_for_node_id()
            .returning(|_, _| Ok(()));
        let service = get_service_with_email_client_and_email_notif_store(
            storage,
            email_client,
            email_notif_storage,
        );
        let res = service.verify_email("123456").await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_get_email_confirmations() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_email_confirmations()
            .returning(|| Ok(vec![]));
        let service = get_service(storage);
        let res = service.get_email_confirmations().await.expect("works");
        assert_eq!(res.len(), 0);
    }

    #[tokio::test]
    async fn test_confirm_email() {
        let mut storage = MockIdentityStoreApiMock::new();
        storage
            .expect_get_or_create_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        let mut email_client = MockEmailClientApi::new();
        email_client
            .expect_register()
            .returning(|_, _, _, _, _| Ok(()));
        let email_notif_storage = MockEmailNotificationStoreApiMock::new();
        let service = get_service_with_email_client_and_email_notif_store(
            storage,
            email_client,
            email_notif_storage,
        );
        let res = service
            .confirm_email(&Email::new("test@example.com").unwrap())
            .await;
        assert!(res.is_ok());
    }

    fn add_identity_proof_block(mut chain: IdentityBlockchain) -> IdentityBlockchain {
        let (proof, data) = signed_identity_proof_test();
        let block = IdentityBlock::create_block_for_identity_proof(
            chain.get_latest_block(),
            &IdentityProofBlockData { proof, data },
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts() - 6,
        )
        .unwrap();
        assert!(chain.try_add_block(block));
        assert!(chain.is_chain_valid());
        chain
    }

    fn get_genesis_chain(identity: Option<Identity>) -> IdentityBlockchain {
        let identity = identity.unwrap_or(empty_identity());
        let chain = IdentityBlockchain::new(
            &identity.into(),
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts() - 7,
        )
        .unwrap();
        add_identity_proof_block(chain)
    }
}
