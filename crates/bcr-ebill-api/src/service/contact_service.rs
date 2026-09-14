use std::sync::Arc;

use async_trait::async_trait;
use bcr_common::core::NodeId;
use bcr_ebill_core::protocol::{Address, EditOptionalFieldMode, Zip};
use bcr_ebill_core::protocol::{
    City, Country, Date, Email, File, Identification, Name, PostalAddress, PublicKey, SecretKey,
    Sha256Hash,
    blockchain::bill::{block::ContactType, participant::BillParticipant},
    crypto::{self, BcrKeys, DeriveKeypair},
};
use bcr_ebill_core::{
    application::{
        ServiceTraitBounds, ValidationError,
        contact::{Contact, validation::validate_create_contact},
        nostr_contact::{NostrContact, NostrPublicKey, TrustLevel},
    },
    protocol::ProtocolValidationError,
};
use bcr_ebill_persistence::{
    ContactStoreApi, FileReferenceStoreApi, company::CompanyStoreApi,
    file_upload::FileUploadStoreApi, identity::IdentityStoreApi, nostr::NostrContactStoreApi,
};
#[cfg(test)]
use mockall::automock;
use uuid::Uuid;

use crate::service::file_reference_helper::{contact_file_context, encrypt_upload_and_track_file};
use crate::{
    Config,
    external::file_storage::FileStorageClientApi,
    get_config,
    service::{
        Result,
        file_server_service::{
            configured_blossom_servers, download_file_with_fallback, resolve_blossom_servers,
        },
        file_upload_service::UploadFileType,
        transport_service::TransportServiceApi,
    },
    util::{self, validate_node_id_network},
};

use log::{debug, error, info};

#[cfg(test)]
impl ServiceTraitBounds for MockContactServiceApi {}

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait ContactServiceApi: ServiceTraitBounds {
    /// Searches contacts and logical contacts for the search term. Both are included by default
    /// and can be disabled by setting the include_logical and include_contact parameters to false.
    async fn search(
        &self,
        search_term: &str,
        include_logical: Option<bool>,
        include_contact: Option<bool>,
    ) -> Result<Vec<Contact>>;
    /// Returns all contacts in short form
    async fn get_contacts(&self) -> Result<Vec<Contact>>;

    /// Returns the contact details for the given node_id
    async fn get_contact(&self, node_id: &NodeId) -> Result<Contact>;

    /// Returns the contact by node id
    async fn get_identity_by_node_id(&self, node_id: &NodeId) -> Result<Option<BillParticipant>>;

    /// Deletes the contact with the given node_id.
    async fn delete(&self, node_id: &NodeId) -> Result<()>;

    /// Updates the contact with the given data.
    async fn update_contact(
        &self,
        node_id: &NodeId,
        name: Option<Name>,
        email: Option<Email>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        date_of_birth_or_registration: EditOptionalFieldMode<Date>,
        country_of_birth_or_registration: EditOptionalFieldMode<Country>,
        city_of_birth_or_registration: EditOptionalFieldMode<City>,
        identification_number: EditOptionalFieldMode<Identification>,
        avatar_file_upload_id: EditOptionalFieldMode<Uuid>,
        proof_document_file_upload_id: EditOptionalFieldMode<Uuid>,
    ) -> Result<()>;

    /// Adds a new contact
    async fn add_contact(
        &self,
        node_id: &NodeId,
        t: ContactType,
        name: Name,
        email: Option<Email>,
        postal_address: Option<PostalAddress>,
        date_of_birth_or_registration: Option<Date>,
        country_of_birth_or_registration: Option<Country>,
        city_of_birth_or_registration: Option<City>,
        identification_number: Option<Identification>,
        avatar_file_upload_id: Option<Uuid>,
        proof_document_file_upload_id: Option<Uuid>,
    ) -> Result<Contact>;

    /// Deanonymize a contact
    async fn deanonymize_contact(
        &self,
        node_id: &NodeId,
        t: ContactType,
        name: Name,
        email: Option<Email>,
        postal_address: Option<PostalAddress>,
        date_of_birth_or_registration: Option<Date>,
        country_of_birth_or_registration: Option<Country>,
        city_of_birth_or_registration: Option<City>,
        identification_number: Option<Identification>,
        avatar_file_upload_id: Option<Uuid>,
        proof_document_file_upload_id: Option<Uuid>,
    ) -> Result<Contact>;

    /// Returns whether a given npub (as hex) is in our contact list.
    async fn is_known_npub(&self, npub: &NostrPublicKey) -> Result<bool>;

    /// Returns the Npubs we want to subscribe to on Nostr.
    async fn get_nostr_npubs(&self) -> Result<Vec<NostrPublicKey>>;

    /// Returns a Nostr contact by node id if we have a trusted one.
    async fn get_nostr_contact_by_node_id(&self, node_id: &NodeId) -> Result<Option<NostrContact>>;

    /// opens and decrypts the attached file from the given contact
    async fn open_and_decrypt_file(
        &self,
        contact: Contact,
        id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>>;

    /// Lists all pending contact shares for the current user's node
    async fn list_pending_contact_shares(
        &self,
        receiver_node_id: &NodeId,
    ) -> Result<Vec<bcr_ebill_persistence::PendingContactShare>>;

    /// Gets a specific pending contact share by ID
    async fn get_pending_contact_share(
        &self,
        id: &str,
    ) -> Result<Option<bcr_ebill_persistence::PendingContactShare>>;

    /// Approves a pending contact share by ID, with options to add to contacts and share back
    ///
    /// # Arguments
    /// * `pending_share_id` - The ID of the pending contact share to approve
    /// * `add_to_contacts` - If true, adds the shared contact to our contacts
    /// * `share_back` - If true, automatically shares the receiver's contact back to the sender
    async fn approve_contact_share(
        &self,
        pending_share_id: &str,
        add_to_contacts: bool,
        share_back: bool,
    ) -> Result<()>;

    /// Rejects a pending contact share by ID (deletes it)
    async fn reject_contact_share(&self, pending_share_id: &str) -> Result<()>;
}

/// The contact service is responsible for managing the local contacts
#[derive(Clone)]
pub struct ContactService {
    store: Arc<dyn ContactStoreApi>,
    file_upload_store: Arc<dyn FileUploadStoreApi>,
    file_upload_client: Arc<dyn FileStorageClientApi>,
    file_reference_store: Arc<dyn FileReferenceStoreApi>,
    identity_store: Arc<dyn IdentityStoreApi>,
    company_store: Arc<dyn CompanyStoreApi>,
    nostr_contact_store: Arc<dyn NostrContactStoreApi>,
    transport_service: Arc<dyn TransportServiceApi>,
    config: Config,
}

impl ContactService {
    pub fn new(
        store: Arc<dyn ContactStoreApi>,
        file_upload_store: Arc<dyn FileUploadStoreApi>,
        file_upload_client: Arc<dyn FileStorageClientApi>,
        file_reference_store: Arc<dyn FileReferenceStoreApi>,
        identity_store: Arc<dyn IdentityStoreApi>,
        company_store: Arc<dyn CompanyStoreApi>,
        nostr_contact_store: Arc<dyn NostrContactStoreApi>,
        transport_service: Arc<dyn TransportServiceApi>,
        config: &Config,
    ) -> Self {
        Self {
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            transport_service,
            config: config.clone(),
        }
    }

    async fn process_upload_file(
        &self,
        upload_id: &Option<Uuid>,
        contact_node_id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        upload_file_type: UploadFileType,
        field_name: &str,
    ) -> Result<Option<File>> {
        if let Some(upload_id) = upload_id {
            debug!("processing upload file for contact {contact_node_id}: {upload_id:?}");
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
                .encrypt_and_save_uploaded_file(
                    file_name,
                    file_bytes,
                    contact_node_id,
                    public_key,
                    signer,
                    field_name,
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
        contact_node_id: &NodeId,
        public_key: &PublicKey,
        signer: &BcrKeys,
        field_name: &str,
    ) -> Result<File> {
        let publisher_node_id = NodeId::new(signer.pub_key(), get_config().bitcoin_network());
        encrypt_upload_and_track_file(
            &self.file_reference_store,
            &self.file_upload_client,
            &self.transport_service,
            &configured_blossom_servers(&get_config().nostr_config),
            file_name,
            file_bytes,
            public_key,
            signer,
            &publisher_node_id,
            contact_file_context(contact_node_id, field_name),
            None,
            "contact",
        )
        .await
    }

    async fn cascade_nostr_contact(&self, contact: &Contact) -> Result<()> {
        let nostr_contact = match self
            .nostr_contact_store
            .by_node_id(&contact.node_id)
            .await?
        {
            Some(nostr_contact) => nostr_contact.merge_contact(contact, None),
            None => NostrContact::from_contact(contact, None)?,
        };
        self.nostr_contact_store.upsert(&nostr_contact).await?;
        Ok(())
    }

    async fn get_blossom_servers(
        &self,
        node_id: &NodeId,
        fallback_relays: &[url::Url],
    ) -> Vec<url::Url> {
        let known_servers = self
            .nostr_contact_store
            .by_node_id(node_id)
            .await
            .ok()
            .flatten()
            .map(|contact| contact.blossom_servers)
            .unwrap_or_default();
        resolve_blossom_servers(&known_servers, fallback_relays)
    }
}

impl ServiceTraitBounds for ContactService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl ContactServiceApi for ContactService {
    async fn search(
        &self,
        search_term: &str,
        include_logical: Option<bool>,
        include_contact: Option<bool>,
    ) -> Result<Vec<Contact>> {
        let mut contacts = if include_contact.unwrap_or(true) {
            self.store.search(search_term).await?
        } else {
            vec![]
        };
        let mut nostr_contacts = if include_logical.unwrap_or(true) {
            let nostr = self
                .nostr_contact_store
                .search(
                    search_term,
                    vec![TrustLevel::Trusted, TrustLevel::Participant],
                )
                .await?;
            let lookup: Vec<NodeId> = contacts.iter().map(|c| c.node_id.clone()).collect();
            nostr
                .into_iter()
                .filter_map(|c| {
                    // only return nostr  contacts that are not in contacts and have a name
                    if !lookup.contains(&c.node_id) {
                        c.into_contact(None)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            vec![]
        };
        contacts.append(&mut nostr_contacts);
        Ok(contacts)
    }

    async fn get_contacts(&self) -> Result<Vec<Contact>> {
        let contact_map = self.store.get_map().await?;
        let contact_list: Vec<Contact> = contact_map.into_values().collect();
        Ok(contact_list)
    }

    async fn get_contact(&self, node_id: &NodeId) -> Result<Contact> {
        validate_node_id_network(node_id)?;
        debug!("getting contact for {node_id}");
        let res = self.store.get(node_id).await?;
        match res {
            None => Err(super::Error::NotFound),
            Some(contact) => Ok(contact),
        }
    }

    async fn get_identity_by_node_id(&self, node_id: &NodeId) -> Result<Option<BillParticipant>> {
        validate_node_id_network(node_id)?;
        let res = self.store.get(node_id).await?;
        res.map(|c| c.try_into().map_err(super::Error::Validation))
            .transpose()
    }

    async fn delete(&self, node_id: &NodeId) -> Result<()> {
        validate_node_id_network(node_id)?;
        self.store.delete(node_id).await?;
        self.nostr_contact_store.delete(node_id).await?;
        Ok(())
    }

    async fn update_contact(
        &self,
        node_id: &NodeId,
        name: Option<Name>,
        email: Option<Email>,
        country: Option<Country>,
        city: Option<City>,
        zip: EditOptionalFieldMode<Zip>,
        address: Option<Address>,
        date_of_birth_or_registration: EditOptionalFieldMode<Date>,
        country_of_birth_or_registration: EditOptionalFieldMode<Country>,
        city_of_birth_or_registration: EditOptionalFieldMode<City>,
        identification_number: EditOptionalFieldMode<Identification>,
        avatar_file_upload_id: EditOptionalFieldMode<Uuid>,
        proof_document_file_upload_id: EditOptionalFieldMode<Uuid>,
    ) -> Result<()> {
        debug!("updating contact with node_id: {node_id}");
        validate_node_id_network(node_id)?;
        let mut contact = match self.store.get(node_id).await? {
            Some(contact) => contact,
            None => {
                return Err(super::Error::NotFound);
            }
        };

        let mut changed = false;

        if let Some(ref name_to_set) = name {
            contact.name = name_to_set.clone();
            changed = true;
        }

        let identity = self.identity_store.get_full().await?;

        // for anonymous contact, we only consider name
        if contact.t != ContactType::Anon {
            if let Some(ref email_to_set) = email {
                contact.email = Some(email_to_set.clone());
                changed = true;
            }

            if let Some(ref mut contact_postal_address) = contact.postal_address {
                if let Some(ref postal_address_city_to_set) = city {
                    contact_postal_address.city = postal_address_city_to_set.clone();
                    changed = true;
                }

                if let Some(ref postal_address_country_to_set) = country {
                    contact_postal_address.country = postal_address_country_to_set.clone();
                    changed = true;
                }

                util::handle_optional_field(&mut contact_postal_address.zip, &zip, &mut changed);

                if let Some(ref postal_address_address_to_set) = address {
                    contact_postal_address.address = postal_address_address_to_set.clone();
                    changed = true;
                }
            } else {
                return Err(super::Error::Validation(ValidationError::InvalidContact(
                    contact.node_id.to_string(),
                )));
            }

            util::handle_optional_field(
                &mut contact.date_of_birth_or_registration,
                &date_of_birth_or_registration,
                &mut changed,
            );

            util::handle_optional_field(
                &mut contact.country_of_birth_or_registration,
                &country_of_birth_or_registration,
                &mut changed,
            );

            util::handle_optional_field(
                &mut contact.city_of_birth_or_registration,
                &city_of_birth_or_registration,
                &mut changed,
            );

            util::handle_optional_field(
                &mut contact.identification_number,
                &identification_number,
                &mut changed,
            );

            let _avatar_file = match avatar_file_upload_id {
                EditOptionalFieldMode::Set(avatar_file_upload_id) => {
                    let avatar_file = self
                        .process_upload_file(
                            &Some(avatar_file_upload_id),
                            node_id,
                            &identity.key_pair.pub_key(),
                            &identity.key_pair,
                            UploadFileType::Picture,
                            "avatar_file",
                        )
                        .await?;

                    if avatar_file.is_some() {
                        contact.avatar_file = avatar_file.clone();
                        changed = true;
                    }
                    avatar_file
                }
                EditOptionalFieldMode::Unset => {
                    // remove the avatar
                    contact.avatar_file = None;
                    changed = true;
                    None
                }
                EditOptionalFieldMode::Ignore => {
                    // nothing to do
                    None
                }
            };

            let _proof_document_file = match proof_document_file_upload_id {
                EditOptionalFieldMode::Set(proof_document_file_upload_id) => {
                    let proof_document_file = self
                        .process_upload_file(
                            &Some(proof_document_file_upload_id),
                            node_id,
                            &identity.key_pair.pub_key(),
                            &identity.key_pair,
                            UploadFileType::Document,
                            "proof_document_file",
                        )
                        .await?;

                    if proof_document_file.is_some() {
                        contact.proof_document_file = proof_document_file.clone();
                        changed = true;
                    }
                    proof_document_file
                }
                EditOptionalFieldMode::Unset => {
                    // remove the proof document
                    contact.proof_document_file = None;
                    changed = true;
                    None
                }
                EditOptionalFieldMode::Ignore => {
                    // nothing to do
                    None
                }
            };

            if !changed {
                log::warn!("Called Contact Change without any changes - returning");
                return Ok(());
            }
        }

        self.store.update(node_id, contact.clone()).await?;
        self.cascade_nostr_contact(&contact).await?;
        debug!("updated contact with node_id: {node_id}");

        Ok(())
    }

    async fn add_contact(
        &self,
        node_id: &NodeId,
        t: ContactType,
        name: Name,
        email: Option<Email>,
        postal_address: Option<PostalAddress>,
        date_of_birth_or_registration: Option<Date>,
        country_of_birth_or_registration: Option<Country>,
        city_of_birth_or_registration: Option<City>,
        identification_number: Option<Identification>,
        avatar_file_upload_id: Option<Uuid>,
        proof_document_file_upload_id: Option<Uuid>,
    ) -> Result<Contact> {
        debug!("creating {t:?} contact with node_id {node_id}");
        validate_node_id_network(node_id)?;
        validate_create_contact(
            t.clone(),
            node_id,
            &email,
            &postal_address,
            get_config().bitcoin_network(),
        )?;

        let nostr_relays = get_config().nostr_config.relays.clone();
        let identity = self.identity_store.get_full().await?;

        let contact = match t {
            ContactType::Company | ContactType::Person => {
                let avatar_file = self
                    .process_upload_file(
                        &avatar_file_upload_id,
                        node_id,
                        &identity.key_pair.pub_key(),
                        &identity.key_pair,
                        UploadFileType::Picture,
                        "avatar_file",
                    )
                    .await?;

                let proof_document_file = self
                    .process_upload_file(
                        &proof_document_file_upload_id,
                        node_id,
                        &identity.key_pair.pub_key(),
                        &identity.key_pair,
                        UploadFileType::Document,
                        "proof_document_file",
                    )
                    .await?;

                Contact {
                    node_id: node_id.clone(),
                    t: t.clone(),
                    name,
                    email,
                    postal_address,
                    date_of_birth_or_registration,
                    country_of_birth_or_registration,
                    city_of_birth_or_registration,
                    identification_number,
                    avatar_file,
                    proof_document_file,
                    nostr_relays,
                    is_logical: false,
                    mint_url: None,
                }
            }
            ContactType::Anon => {
                Contact {
                    node_id: node_id.clone(),
                    t: t.clone(),
                    name,
                    email: None,
                    postal_address: None,
                    date_of_birth_or_registration: None,
                    country_of_birth_or_registration: None,
                    city_of_birth_or_registration: None,
                    identification_number: None,
                    avatar_file: None,
                    proof_document_file: None,
                    nostr_relays: get_config().nostr_config.relays.clone(), // Use the configured relays for now
                    is_logical: false,
                    mint_url: None,
                }
            }
        };

        self.store.insert(node_id, contact.clone()).await?;
        self.cascade_nostr_contact(&contact).await?;
        debug!("contact {t:?} with node_id {node_id} created");
        Ok(contact)
    }

    async fn deanonymize_contact(
        &self,
        node_id: &NodeId,
        t: ContactType,
        name: Name,
        email: Option<Email>,
        postal_address: Option<PostalAddress>,
        date_of_birth_or_registration: Option<Date>,
        country_of_birth_or_registration: Option<Country>,
        city_of_birth_or_registration: Option<City>,
        identification_number: Option<Identification>,
        avatar_file_upload_id: Option<Uuid>,
        proof_document_file_upload_id: Option<Uuid>,
    ) -> Result<Contact> {
        debug!("de-anonymizing {t:?} contact with node_id {node_id}");
        validate_node_id_network(node_id)?;
        validate_create_contact(
            t.clone(),
            node_id,
            &email,
            &postal_address,
            get_config().bitcoin_network(),
        )?;

        // can't de-anonymize to an anonymous contact
        if t == ContactType::Anon {
            return Err(super::Error::Validation(
                ProtocolValidationError::InvalidContactType.into(),
            ));
        }

        let existing_anon_contact = match self.store.get(node_id).await? {
            Some(existing_anon_contact) => existing_anon_contact,
            None => {
                return Err(super::Error::NotFound);
            }
        };

        // if the existing contact is not anonymous, the action is not valid
        if existing_anon_contact.t != ContactType::Anon {
            return Err(super::Error::Validation(ValidationError::InvalidContact(
                node_id.to_string(),
            )));
        }

        let identity_keys = self.identity_store.get_key_pair().await?;
        let identity_public_key = identity_keys.pub_key();

        let avatar_file = self
            .process_upload_file(
                &avatar_file_upload_id,
                node_id,
                &identity_public_key,
                &identity_keys,
                UploadFileType::Picture,
                "avatar_file",
            )
            .await?;

        let proof_document_file = self
            .process_upload_file(
                &proof_document_file_upload_id,
                node_id,
                &identity_public_key,
                &identity_keys,
                UploadFileType::Document,
                "proof_document_file",
            )
            .await?;

        let contact = Contact {
            node_id: node_id.clone(),
            t: t.clone(),
            name,
            email,
            postal_address,
            date_of_birth_or_registration,
            country_of_birth_or_registration,
            city_of_birth_or_registration,
            identification_number,
            avatar_file,
            proof_document_file,
            nostr_relays: self.config.nostr_config.relays.clone(),
            is_logical: false,
            mint_url: None,
        };

        debug!("contact {t:?} with node_id {node_id} created");
        self.store.update(node_id, contact.clone()).await?;
        self.cascade_nostr_contact(&contact).await?;
        debug!("deanonymized contact with node_id: {node_id}");
        Ok(contact)
    }

    async fn is_known_npub(&self, npub: &NostrPublicKey) -> Result<bool> {
        Ok(!self.config.nostr_config.only_known_contacts
            || self
                .nostr_contact_store
                .by_npub(npub)
                .await?
                .map(|c| c.trust_level != TrustLevel::None)
                .unwrap_or(false))
    }

    /// Returns the Npubs we want to subscribe to on Nostr.
    async fn get_nostr_npubs(&self) -> Result<Vec<NostrPublicKey>> {
        Ok(self
            .nostr_contact_store
            .get_npubs(vec![TrustLevel::Trusted, TrustLevel::Participant])
            .await?)
    }

    /// Returns a Nostr contact by node id if we have a trusted or participant one.
    async fn get_nostr_contact_by_node_id(&self, node_id: &NodeId) -> Result<Option<NostrContact>> {
        validate_node_id_network(node_id)?;
        match self.nostr_contact_store.by_node_id(node_id).await {
            Ok(Some(c)) if c.trust_level != TrustLevel::None => Ok(Some(c)),
            _ => Ok(None),
        }
    }

    async fn open_and_decrypt_file(
        &self,
        contact: Contact,
        node_id: &NodeId,
        file_name: &Name,
        private_key: &SecretKey,
    ) -> Result<Vec<u8>> {
        debug!("getting file {file_name} for contact with id: {node_id}",);
        validate_node_id_network(node_id)?;
        let mut file = None;
        if let Some(avatar_file) = contact.avatar_file
            && &avatar_file.name == file_name
        {
            file = Some(avatar_file);
        }

        if let Some(proof_document_file) = contact.proof_document_file
            && &proof_document_file.name == file_name
        {
            file = Some(proof_document_file);
        }

        if let Some(file) = file {
            let blossom_servers = self
                .get_blossom_servers(node_id, &contact.nostr_relays)
                .await;
            let file_bytes = download_file_with_fallback(
                self.file_upload_client.as_ref(),
                Some(&self.file_reference_store),
                Some(&self.transport_service),
                &blossom_servers,
                &file.hash,
                &file.nostr_hash,
            )
            .await?;
            let decrypted = crypto::decrypt_ecies(&file_bytes, private_key)?;
            let file_hash = Sha256Hash::from_bytes(&decrypted);
            if file_hash != file.hash {
                error!("Hash for contact file {file_name} did not match uploaded file");
                return Err(super::Error::NotFound);
            }
            Ok(decrypted)
        } else {
            Err(super::Error::NotFound)
        }
    }

    async fn list_pending_contact_shares(
        &self,
        receiver_node_id: &NodeId,
    ) -> Result<Vec<bcr_ebill_persistence::PendingContactShare>> {
        validate_node_id_network(receiver_node_id)?;
        // By default, only return incoming shares (not the ones we created when sharing)
        Ok(self
            .nostr_contact_store
            .list_pending_shares_by_receiver_and_direction(
                receiver_node_id,
                bcr_ebill_persistence::ShareDirection::Incoming,
            )
            .await?)
    }

    async fn get_pending_contact_share(
        &self,
        id: &str,
    ) -> Result<Option<bcr_ebill_persistence::PendingContactShare>> {
        Ok(self.nostr_contact_store.get_pending_share(id).await?)
    }

    async fn approve_contact_share(
        &self,
        pending_share_id: &str,
        add_to_contacts: bool,
        share_back: bool,
    ) -> Result<()> {
        // Get the pending share
        let pending_share = self
            .nostr_contact_store
            .get_pending_share(pending_share_id)
            .await?
            .ok_or(super::Error::NotFound)?;

        let node_id = &pending_share.node_id;
        let contact = &pending_share.contact;
        let private_key = &pending_share.contact_private_key;

        // Add/update the contact in the contact store if requested
        if add_to_contacts {
            if self.store.get(node_id).await?.is_some() {
                self.store.update(node_id, contact.clone()).await?;
            } else {
                self.store.insert(node_id, contact.clone()).await?;
            }

            // Update NostrContact with the private key
            let upsert = if let Ok(Some(nostr_contact)) =
                self.nostr_contact_store.by_node_id(node_id).await
            {
                nostr_contact.merge_contact(contact, Some(*private_key))
            } else {
                NostrContact::from_contact(contact, Some(*private_key))?
            };
            self.nostr_contact_store.upsert(&upsert).await?;

            info!(
                "approved contact share for {} and added to contacts",
                node_id
            );
        } else {
            info!(
                "approved contact share for {} without adding to contacts",
                node_id
            );
        }

        // Delete the pending share
        self.nostr_contact_store
            .delete_pending_share(pending_share_id)
            .await?;

        // If share_back is true, share our contact back to the sender
        if share_back {
            let sender_node_id = &pending_share.sender_node_id;
            let receiver_node_id = &pending_share.receiver_node_id;

            // First, try to match with the user's identity
            let receiver_identity = self.identity_store.get_full().await?;

            let keys = if &receiver_identity.identity.node_id == receiver_node_id {
                // Share back as identity
                let derived_keys = receiver_identity.key_pair.derive_identity_keypair()?;
                BcrKeys::from_private_key(&derived_keys.secret_key())
            } else if self.company_store.exists(receiver_node_id).await {
                // Share back as company
                let company_keys = self.company_store.get_key_pair(receiver_node_id).await?;
                let derived_keys = company_keys.derive_company_keypair()?;
                BcrKeys::from_private_key(&derived_keys.secret_key())
            } else {
                // receiver_node_id doesn't match identity or any company
                // this should never happen as one of our identities was the receiver
                return Err(super::Error::Validation(
                    bcr_ebill_core::application::ValidationError::Protocol(
                        bcr_ebill_core::protocol::ProtocolValidationError::InvalidContactType,
                    ),
                ));
            };

            // Share the receiver's contact back to the sender, including the initial_share_id
            // from the sender as share_back_pending_id so the sender can auto-accept
            info!("sharing back contact to {}", sender_node_id);
            self.transport_service
                .contact_transport()
                .share_contact_details_keys(
                    sender_node_id,
                    receiver_node_id,
                    &keys,
                    pending_share.initial_share_id.clone(),
                )
                .await?;
        }

        Ok(())
    }

    async fn reject_contact_share(&self, pending_share_id: &str) -> Result<()> {
        self.nostr_contact_store
            .delete_pending_share(pending_share_id)
            .await?;
        info!("rejected contact share {}", pending_share_id);
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{
        external::file_storage::MockFileStorageClientApi,
        get_config,
        service::{
            Error, bill_service::test_utils::get_baseline_identity,
            transport_service::MockTransportServiceApi,
        },
        tests::tests::{
            MockCompanyStoreApiMock, MockContactStoreApiMock, MockFileReferenceStoreApiMock,
            MockFileUploadStoreApiMock, MockIdentityStoreApiMock, MockNostrContactStore,
            NODE_ID_TEST_STR, empty_address, init_test_cfg, node_id_test, node_id_test_other,
        },
    };
    use bcr_ebill_core::{
        application::nostr_contact::HandshakeStatus,
        protocol::{Sha256Hash, crypto::BcrKeys, file_reference::FileReference},
    };
    use bcr_ebill_persistence::PendingContactShare;
    use std::{collections::HashMap, str::FromStr};
    use uuid::Uuid;

    pub fn get_baseline_contact() -> Contact {
        Contact {
            t: ContactType::Person,
            node_id: node_id_test(),
            name: Name::new("some_name").unwrap(),
            email: Some(Email::new("some_mail@example.com").unwrap()),
            postal_address: Some(empty_address()),
            date_of_birth_or_registration: None,
            country_of_birth_or_registration: None,
            city_of_birth_or_registration: None,
            identification_number: None,
            avatar_file: None,
            proof_document_file: None,
            nostr_relays: vec![],
            is_logical: false,
            mint_url: None,
        }
    }

    pub fn get_baseline_nostr_contact() -> NostrContact {
        NostrContact {
            npub: node_id_test_other().npub(),
            node_id: node_id_test_other(),
            name: Some(Name::new("Other Contact").unwrap()),
            relays: vec![],
            blossom_servers: vec![],
            trust_level: TrustLevel::Participant,
            handshake_status: HandshakeStatus::None,
            contact_private_key: None,
            mint_url: None,
        }
    }

    fn get_service(
        mock_storage: MockContactStoreApiMock,
        mock_file_upload_storage: MockFileUploadStoreApiMock,
        mock_file_upload_client: MockFileStorageClientApi,
        mock_file_reference_store: MockFileReferenceStoreApiMock,
        mock_identity_storage: MockIdentityStoreApiMock,
        mock_company_storage: MockCompanyStoreApiMock,
        mock_nostr_contact_store: MockNostrContactStore,
        mock_transport_service: MockTransportServiceApi,
    ) -> ContactService {
        ContactService::new(
            Arc::new(mock_storage),
            Arc::new(mock_file_upload_storage),
            Arc::new(mock_file_upload_client),
            Arc::new(mock_file_reference_store),
            Arc::new(mock_identity_storage),
            Arc::new(mock_company_storage),
            Arc::new(mock_nostr_contact_store),
            Arc::new(mock_transport_service),
            get_config(),
        )
    }

    fn get_storages() -> (
        MockContactStoreApiMock,
        MockFileUploadStoreApiMock,
        MockFileStorageClientApi,
        MockFileReferenceStoreApiMock,
        MockIdentityStoreApiMock,
        MockCompanyStoreApiMock,
        MockNostrContactStore,
        MockTransportServiceApi,
    ) {
        (
            MockContactStoreApiMock::new(),
            MockFileUploadStoreApiMock::new(),
            MockFileStorageClientApi::new(),
            MockFileReferenceStoreApiMock::new(),
            MockIdentityStoreApiMock::new(),
            MockCompanyStoreApiMock::new(),
            MockNostrContactStore::new(),
            MockTransportServiceApi::new(),
        )
    }

    #[tokio::test]
    async fn get_contacts_baseline() {
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        ) = get_storages();
        store.expect_get_map().returning(|| {
            let mut contact = get_baseline_contact();
            contact.name = Name::new("Minka").unwrap();
            let mut map = HashMap::new();
            map.insert(node_id_test(), contact);
            Ok(map)
        });
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        )
        .get_contacts()
        .await;
        assert!(result.is_ok());
        assert_eq!(
            result.as_ref().unwrap().first().unwrap().name,
            Name::new("Minka").unwrap()
        );
        assert_eq!(
            result.as_ref().unwrap().first().unwrap().node_id,
            node_id_test()
        );
    }

    #[tokio::test]
    async fn get_identity_by_node_id_baseline() {
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        ) = get_storages();
        store.expect_get().returning(|_| {
            let mut contact = get_baseline_contact();
            contact.name = Name::new("Minka").unwrap();
            Ok(Some(contact))
        });
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        )
        .get_identity_by_node_id(&node_id_test())
        .await;
        assert!(result.is_ok());
        assert_eq!(
            result.as_ref().unwrap().as_ref().unwrap().name(),
            Some(Name::new("Minka").unwrap())
        );
    }

    #[tokio::test]
    async fn wrong_network_failures() {
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        ) = get_storages();
        let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        );

        assert!(
            service
                .get_identity_by_node_id(&mainnet_node_id)
                .await
                .is_err()
        );
        assert!(service.delete(&mainnet_node_id).await.is_err());
        assert!(service.get_contact(&mainnet_node_id).await.is_err());
        assert!(
            service
                .get_nostr_contact_by_node_id(&mainnet_node_id)
                .await
                .is_err()
        );
        assert!(
            service
                .update_contact(
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
                )
                .await
                .is_err()
        );
        assert!(
            service
                .add_contact(
                    &mainnet_node_id,
                    ContactType::Person,
                    Name::new("name").unwrap(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .is_err()
        );
        assert!(
            service
                .deanonymize_contact(
                    &mainnet_node_id,
                    ContactType::Person,
                    Name::new("name").unwrap(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_contact() {
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact,
            notification,
        ) = get_storages();
        store.expect_delete().returning(|_| Ok(())).once();
        nostr_contact.expect_delete().returning(|_| Ok(())).once();
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        )
        .delete(&node_id_test())
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_contact_calls_store() {
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_full()
            .returning(|| Ok(get_baseline_identity()));
        store.expect_get().returning(|_| {
            let contact = get_baseline_contact();
            Ok(Some(contact))
        });
        store.expect_update().returning(|_, _| Ok(()));

        // and cascades to nostr contacts
        nostr_contact
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact.expect_upsert().returning(|_| Ok(())).once();

        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        )
        .update_contact(
            &node_id_test(),
            Some(Name::new("new_name").unwrap()),
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
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_contact_upload_avatar_uses_identity_node_id_for_publish() {
        init_test_cfg();
        let (
            mut store,
            mut file_upload_store,
            mut file_upload_client,
            mut file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact,
            mut transport,
        ) = get_storages();

        let identity = get_baseline_identity();
        let expected_publisher_node_id =
            NodeId::new(identity.key_pair.pub_key(), get_config().bitcoin_network());

        identity_store
            .expect_get_full()
            .returning(move || Ok(identity.clone()));
        store.expect_get().returning(|_| {
            let contact = get_baseline_contact();
            Ok(Some(contact))
        });
        store.expect_update().returning(|_, _| Ok(()));

        nostr_contact
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact.expect_upsert().returning(|_| Ok(())).once();

        file_upload_store
            .expect_read_temp_upload_file()
            .returning(|_| Ok((Name::new("avatar.png").unwrap(), vec![1, 2, 3])));

        file_upload_client.expect_upload().returning(|_, _| {
            Ok(bitcoin::hashes::sha256::Hash::from_str(
                "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
            )
            .unwrap())
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
                        "0000000000000000000000000000000000000000000000000000000000000000",
                    )
                    .unwrap(),
                    None,
                ))
            });

        transport
            .expect_publish_file_metadata()
            .withf(move |node_id, _plaintext, _encrypted, _urls, _mime| {
                *node_id == expected_publisher_node_id
            })
            .returning(|_, _, _, _, _| Ok(()))
            .times(1);

        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        )
        .update_contact(
            &node_id_test(),
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
            EditOptionalFieldMode::Set(Uuid::new_v4()),
            EditOptionalFieldMode::Ignore,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn add_contact_calls_store() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_full()
            .returning(|| Ok(get_baseline_identity()));
        store.expect_insert().returning(|_, _| Ok(()));

        // and cascades to nostr contacts
        nostr_contact
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact.expect_upsert().returning(|_| Ok(())).once();

        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        )
        .add_contact(
            &node_id_test(),
            ContactType::Person,
            Name::new("some_name").unwrap(),
            Some(Email::new("some_email@example.com").unwrap()),
            Some(empty_address()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn add_anon_contact_calls_store() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact_store,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_full()
            .returning(|| Ok(get_baseline_identity()));
        store.expect_insert().returning(|_, _| Ok(()));

        // and cascades to nostr contacts
        nostr_contact_store
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact_store
            .expect_upsert()
            .returning(|_| Ok(()))
            .once();

        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            notification,
        )
        .add_contact(
            &node_id_test(),
            ContactType::Anon,
            Name::new("some_name").unwrap(),
            Some(Email::new("some_email@example.com").unwrap()),
            Some(empty_address()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_ok());
        // email is not set, even if it's provided
        assert!(result.as_ref().unwrap().email.is_none());
    }

    #[tokio::test]
    async fn deanonymize_contact_calls_store() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact_store,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        store.expect_update().returning(|_, _| Ok(()));
        store.expect_get().returning(|_| {
            let mut contact = get_baseline_contact();
            contact.t = ContactType::Anon;
            Ok(Some(contact))
        });
        nostr_contact_store
            .expect_by_node_id()
            .returning(|_| Ok(None));
        nostr_contact_store.expect_upsert().returning(|_| Ok(()));
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            notification,
        )
        .deanonymize_contact(
            &node_id_test(),
            ContactType::Person,
            Name::new("some_name").unwrap(),
            Some(Email::new("some_email@example.com").unwrap()),
            Some(empty_address()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn deanonymize_contact_of_non_anon_fails() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            nostr_contact_store,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        store.expect_update().returning(|_, _| Ok(()));
        store.expect_get().returning(|_| {
            let contact = get_baseline_contact();
            Ok(Some(contact))
        });
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            notification,
        )
        .deanonymize_contact(
            &node_id_test(),
            ContactType::Person,
            Name::new("some_name").unwrap(),
            Some(Email::new("some_email@example.com").unwrap()),
            Some(empty_address()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
        if let Err(Error::Validation(ValidationError::InvalidContact(node_id))) = result {
            assert_eq!(node_id, NODE_ID_TEST_STR.to_owned());
        } else {
            panic!("wrong error");
        }
    }

    #[tokio::test]
    async fn deanonymize_contact_with_new_anon_fails() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            nostr_contact_store,
            notification,
        ) = get_storages();
        identity_store
            .expect_get_key_pair()
            .returning(|| Ok(BcrKeys::new()));
        store.expect_update().returning(|_, _| Ok(()));
        store.expect_get().returning(|_| {
            let mut contact = get_baseline_contact();
            contact.t = ContactType::Anon;
            Ok(Some(contact))
        });
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            notification,
        )
        .deanonymize_contact(
            &node_id_test(),
            ContactType::Anon,
            Name::new("some_name").unwrap(),
            Some(Email::new("some_email@example.com").unwrap()),
            Some(empty_address()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
        if let Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidContactType,
        ))) = result
        {
            // fine
        } else {
            panic!("wrong error");
        }
    }

    #[tokio::test]
    async fn is_known_npub_calls_store() {
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact,
            notification,
        ) = get_storages();
        let pub_key = node_id_test().npub();
        nostr_contact.expect_by_npub().returning(|_| {
            Ok(Some(NostrContact {
                npub: node_id_test().npub(),
                node_id: node_id_test(),
                name: None,
                relays: vec![],
                blossom_servers: vec![],
                trust_level: TrustLevel::Participant,
                handshake_status: HandshakeStatus::None,
                contact_private_key: None,
                mint_url: None,
            }))
        });
        let result = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            notification,
        )
        .is_known_npub(&pub_key)
        .await;
        assert!(result.is_ok());
        assert!(result.as_ref().unwrap());
    }

    fn pending_contact_share(pending_share_id: &str) -> PendingContactShare {
        PendingContactShare {
            id: pending_share_id.to_string(),
            node_id: node_id_test_other(),
            contact: get_baseline_contact(),
            sender_node_id: node_id_test(),
            contact_private_key: BcrKeys::new().get_private_key(),
            receiver_node_id: node_id_test(),
            received_at: bcr_ebill_core::protocol::Timestamp::now(),
            direction: bcr_ebill_persistence::ShareDirection::Incoming,
            initial_share_id: Some("initial_id".to_string()),
        }
    }

    #[tokio::test]
    async fn approve_contact_share_adds_contact_without_share_back() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact,
            transport,
        ) = get_storages();

        let pending_share_id = "test_share_id";
        let pending_share = pending_contact_share(pending_share_id);
        let shared_node_id = pending_share.node_id.clone();

        // Mock getting the pending share
        nostr_contact
            .expect_get_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(move |_| Ok(Some(pending_share.clone())));

        // Expect contact to be inserted
        store
            .expect_get()
            .with(mockall::predicate::eq(shared_node_id.clone()))
            .returning(|_| Ok(None));
        store.expect_insert().returning(|_, _| Ok(())).once();

        // Expect NostrContact to be upserted
        nostr_contact
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact.expect_upsert().returning(|_| Ok(())).once();

        // Expect pending share to be deleted
        nostr_contact
            .expect_delete_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(|_| Ok(()))
            .once();

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        );

        let result = service
            .approve_contact_share(pending_share_id, true, false)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn approve_contact_share_adds_contact_with_share_back() {
        init_test_cfg();
        let (
            mut store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact,
            mut transport,
        ) = get_storages();

        let receiver_identity = get_baseline_identity();

        let pending_share_id = "test_share_id";
        let pending_share = pending_contact_share(pending_share_id);
        let shared_node_id = pending_share.node_id.clone();

        // Mock getting the pending share
        nostr_contact
            .expect_get_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(move |_| Ok(Some(pending_share.clone())));

        // Expect contact to be inserted
        store
            .expect_get()
            .with(mockall::predicate::eq(shared_node_id.clone()))
            .returning(|_| Ok(None));
        store.expect_insert().returning(|_, _| Ok(())).once();

        // Expect NostrContact to be upserted
        nostr_contact
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();
        nostr_contact.expect_upsert().returning(|_| Ok(())).once();

        // Expect pending share to be deleted
        nostr_contact
            .expect_delete_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(|_| Ok(()))
            .once();

        // Expect identity to be fetched for share back
        identity_store
            .expect_get_full()
            .returning(move || Ok(receiver_identity.clone()))
            .once();

        // Share back will use identity since receiver_node_id matches identity.node_id
        // So company_store.exists() will NOT be called

        // Expect share back to be called
        let mut contact_transport =
            crate::service::transport_service::MockContactTransportServiceApi::new();
        contact_transport
            .expect_share_contact_details_keys()
            .times(1)
            .returning(|_, _, _, _| Ok(()));
        transport
            .expect_contact_transport()
            .times(1)
            .return_const(Arc::new(contact_transport));

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        );

        let result = service
            .approve_contact_share(pending_share_id, true, true)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn approve_contact_share_shares_back_without_adding_contact() {
        init_test_cfg();
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            mut identity_store,
            company_store,
            mut nostr_contact,
            mut transport,
        ) = get_storages();

        let receiver_identity = get_baseline_identity();
        let pending_share_id = "test_share_id";
        let pending_share = pending_contact_share(pending_share_id);

        // Mock getting the pending share
        nostr_contact
            .expect_get_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(move |_| Ok(Some(pending_share.clone())));

        // Expect pending share to be deleted
        nostr_contact
            .expect_delete_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(|_| Ok(()))
            .once();

        // Expect identity to be fetched for share back
        identity_store
            .expect_get_full()
            .returning(move || Ok(receiver_identity.clone()))
            .once();

        // Share back will use identity since receiver_node_id matches identity.node_id
        // So company_store.exists() will NOT be called

        // Expect share back to be called
        let mut contact_transport =
            crate::service::transport_service::MockContactTransportServiceApi::new();
        contact_transport
            .expect_share_contact_details_keys()
            .times(1)
            .returning(|_, _, _, _| Ok(()));
        transport
            .expect_contact_transport()
            .times(1)
            .return_const(Arc::new(contact_transport));

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        );

        let result = service
            .approve_contact_share(pending_share_id, false, true)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn approve_contact_share_does_nothing_when_both_false() {
        init_test_cfg();
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact,
            transport,
        ) = get_storages();

        let pending_share_id = "test_share_id";
        let pending_share = pending_contact_share(pending_share_id);

        // Mock getting the pending share
        nostr_contact
            .expect_get_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(move |_| Ok(Some(pending_share.clone())));

        // Expect pending share to be deleted (only operation that should happen)
        nostr_contact
            .expect_delete_pending_share()
            .with(mockall::predicate::eq(pending_share_id))
            .returning(|_| Ok(()))
            .once();

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact,
            transport,
        );

        let result = service
            .approve_contact_share(pending_share_id, false, false)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn get_blossom_servers_prefers_known_contact_servers() {
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact_store,
            transport,
        ) = get_storages();
        let expected = url::Url::parse("https://known-blossom.example.com").unwrap();

        let expected_for_mock = expected.clone();
        nostr_contact_store
            .expect_by_node_id()
            .returning(move |_| {
                let mut contact = get_baseline_nostr_contact();
                contact.blossom_servers = vec![expected_for_mock.clone()];
                Ok(Some(contact))
            })
            .once();

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            transport,
        );

        let result = service
            .get_blossom_servers(
                &node_id_test_other(),
                &[url::Url::parse("wss://relay.example.com").unwrap()],
            )
            .await;

        assert_eq!(result, vec![expected]);
    }

    #[tokio::test]
    async fn get_blossom_servers_falls_back_to_first_contact_relay() {
        let (
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            mut nostr_contact_store,
            transport,
        ) = get_storages();

        nostr_contact_store
            .expect_by_node_id()
            .returning(|_| Ok(None))
            .once();

        let service = get_service(
            store,
            file_upload_store,
            file_upload_client,
            file_reference_store,
            identity_store,
            company_store,
            nostr_contact_store,
            transport,
        );

        let result = service
            .get_blossom_servers(
                &node_id_test_other(),
                &[url::Url::parse("wss://relay.example.com").unwrap()],
            )
            .await;

        assert_eq!(
            result,
            vec![url::Url::parse("https://relay.example.com/").unwrap()]
        );
    }
}
