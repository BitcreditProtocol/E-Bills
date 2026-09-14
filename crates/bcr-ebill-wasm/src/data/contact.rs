use bcr_common::core::NodeId;
use bcr_ebill_api::service::Error;
use bcr_ebill_core::{
    application::{ValidationError, contact::Contact},
    protocol::{
        City, Country, Date, Email, Identification, Name, Timestamp, blockchain::bill::ContactType,
    },
};
use bcr_ebill_persistence::PendingContactShare;
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use crate::data::{CreateOptionalPostalAddressWeb, CreatePostalAddressWeb};

use super::{FileWeb, PostalAddressWeb};

#[derive(Tsify, Debug, Serialize)]
pub struct ContactsResponse {
    pub contacts: Vec<ContactWeb>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct NewContactPayload {
    pub t: u64,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: Option<CreatePostalAddressWeb>,
    pub date_of_birth_or_registration: Option<String>,
    pub country_of_birth_or_registration: Option<String>,
    pub city_of_birth_or_registration: Option<String>,
    pub identification_number: Option<String>,
    pub avatar_file_upload_id: Option<String>,
    pub proof_document_file_upload_id: Option<String>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct EditContactPayload {
    #[tsify(type = "string")]
    pub node_id: NodeId,
    pub name: Option<String>,
    pub email: Option<String>,
    pub postal_address: CreateOptionalPostalAddressWeb,
    pub date_of_birth_or_registration: Option<String>,
    pub country_of_birth_or_registration: Option<String>,
    pub city_of_birth_or_registration: Option<String>,
    pub identification_number: Option<String>,
    pub avatar_file_upload_id: Option<String>,
    pub proof_document_file_upload_id: Option<String>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct SearchContactsPayload {
    pub search_term: String,
    pub include_logical: Option<bool>,
    pub include_contact: Option<bool>,
}

#[wasm_bindgen]
#[repr(u8)]
#[derive(
    Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr, PartialEq, Eq,
)]
pub enum ContactTypeWeb {
    Person = 0,
    Company = 1,
    Anon = 2,
}

impl TryFrom<u64> for ContactTypeWeb {
    type Error = Error;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        Ok(ContactType::try_from(value)
            .map_err(|e| Self::Error::Validation(ValidationError::Protocol(e)))?
            .into())
    }
}

impl From<ContactType> for ContactTypeWeb {
    fn from(val: ContactType) -> Self {
        match val {
            ContactType::Person => ContactTypeWeb::Person,
            ContactType::Company => ContactTypeWeb::Company,
            ContactType::Anon => ContactTypeWeb::Anon,
        }
    }
}

impl From<ContactTypeWeb> for ContactType {
    fn from(value: ContactTypeWeb) -> Self {
        match value {
            ContactTypeWeb::Person => ContactType::Person,
            ContactTypeWeb::Company => ContactType::Company,
            ContactTypeWeb::Anon => ContactType::Anon,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct ContactWeb {
    pub t: ContactTypeWeb,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string | undefined")]
    pub email: Option<Email>,
    pub postal_address: Option<PostalAddressWeb>,
    #[tsify(type = "string")]
    pub date_of_birth_or_registration: Option<Date>,
    #[tsify(type = "string | undefined")]
    pub country_of_birth_or_registration: Option<Country>,
    #[tsify(type = "string | undefined")]
    pub city_of_birth_or_registration: Option<City>,
    #[tsify(type = "string | undefined")]
    pub identification_number: Option<Identification>,
    pub avatar_file: Option<FileWeb>,
    pub proof_document_file: Option<FileWeb>,
    #[tsify(type = "string[]")]
    pub nostr_relays: Vec<url::Url>,
    pub is_logical: bool,
}

impl From<Contact> for ContactWeb {
    fn from(val: Contact) -> Self {
        ContactWeb {
            t: val.t.into(),
            node_id: val.node_id,
            name: val.name,
            email: val.email,
            postal_address: val.postal_address.map(|pa| pa.into()),
            date_of_birth_or_registration: val.date_of_birth_or_registration,
            country_of_birth_or_registration: val.country_of_birth_or_registration,
            city_of_birth_or_registration: val.city_of_birth_or_registration,
            identification_number: val.identification_number,
            avatar_file: val.avatar_file.map(|f| f.into()),
            proof_document_file: val.proof_document_file.map(|f| f.into()),
            nostr_relays: val.nostr_relays,
            is_logical: val.is_logical,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct PendingContactShareWeb {
    pub id: String,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    pub contact: ContactWeb,
    #[tsify(type = "string")]
    pub sender_node_id: NodeId,
    #[tsify(type = "string")]
    pub receiver_node_id: NodeId,
    #[tsify(type = "number")]
    pub received_at: Timestamp,
}

impl From<PendingContactShare> for PendingContactShareWeb {
    fn from(val: PendingContactShare) -> Self {
        PendingContactShareWeb {
            id: val.id,
            node_id: val.node_id,
            contact: val.contact.into(),
            sender_node_id: val.sender_node_id,
            receiver_node_id: val.receiver_node_id,
            received_at: val.received_at,
        }
    }
}

#[derive(Tsify, Debug, Serialize)]
pub struct PendingContactSharesResponse {
    pub pending_shares: Vec<PendingContactShareWeb>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ApproveContactSharePayload {
    pub pending_share_id: String,
    pub add_to_contacts: bool,
    pub share_back: bool,
}
