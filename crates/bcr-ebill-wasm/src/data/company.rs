use bcr_common::core::NodeId;
use bcr_ebill_core::{
    application::{
        ValidationError,
        company::{Company, CompanySignatory, CompanySignatoryStatus, CompanyStatus},
        contact::Contact,
    },
    protocol::{
        City, Country, Date, Email, Identification, Name, Timestamp,
        blockchain::bill::block::ContactType,
    },
};
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use crate::data::{
    CreateOptionalPostalAddressWeb, CreatePostalAddressWeb, identity::IdentityEmailConfirmationWeb,
};

use super::{FileWeb, PostalAddressWeb, contact::ContactTypeWeb};

#[derive(Tsify, Debug, Serialize)]
pub struct CompaniesResponse {
    pub companies: Vec<CompanyWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum CompanyStatusWeb {
    Invited,
    Active,
    None,
}

impl From<CompanyStatus> for CompanyStatusWeb {
    fn from(value: CompanyStatus) -> Self {
        match value {
            CompanyStatus::Invited => CompanyStatusWeb::Invited,
            CompanyStatus::Active => CompanyStatusWeb::Active,
            CompanyStatus::None => CompanyStatusWeb::None,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct CompanyWeb {
    #[tsify(type = "string")]
    pub id: NodeId,
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string | undefined")]
    pub country_of_registration: Option<Country>,
    #[tsify(type = "string | undefined")]
    pub city_of_registration: Option<City>,
    pub postal_address: PostalAddressWeb,
    #[tsify(type = "string")]
    pub email: Email,
    #[tsify(type = "string | undefined")]
    pub registration_number: Option<Identification>,
    #[tsify(type = "string | undefined")]
    pub registration_date: Option<Date>,
    pub proof_of_registration_file: Option<FileWeb>,
    pub logo_file: Option<FileWeb>,
    pub signatories: Vec<CompanySignatoryWeb>,
    #[tsify(type = "number")]
    pub creation_time: Timestamp,
    pub status: CompanyStatusWeb,
}

impl From<Company> for CompanyWeb {
    fn from(val: Company) -> Self {
        CompanyWeb {
            id: val.id,
            name: val.name,
            country_of_registration: val.country_of_registration,
            city_of_registration: val.city_of_registration,
            postal_address: val.postal_address.into(),
            email: val.email,
            registration_number: val.registration_number,
            registration_date: val.registration_date,
            proof_of_registration_file: val.proof_of_registration_file.map(|f| f.into()),
            logo_file: val.logo_file.map(|f| f.into()),
            signatories: val.signatories.into_iter().map(|s| s.into()).collect(),
            creation_time: val.creation_time,
            status: val.status.into(),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum CompanySignatoryStatusWeb {
    Invited {
        #[tsify(type = "number")]
        ts: Timestamp,
        #[tsify(type = "string")]
        inviter: NodeId,
    },
    InviteAccepted {
        #[tsify(type = "number")]
        ts: Timestamp,
    },
    InviteRejected {
        #[tsify(type = "number")]
        ts: Timestamp,
    },
    InviteAcceptedIdentityProven {
        #[tsify(type = "number")]
        ts: Timestamp,
        confirmation: IdentityEmailConfirmationWeb,
    },
    Removed {
        #[tsify(type = "number")]
        ts: Timestamp,
        #[tsify(type = "string")]
        remover: NodeId,
    },
}

impl From<CompanySignatoryStatus> for CompanySignatoryStatusWeb {
    fn from(value: CompanySignatoryStatus) -> Self {
        match value {
            CompanySignatoryStatus::Invited { ts, inviter } => {
                CompanySignatoryStatusWeb::Invited { ts, inviter }
            }
            CompanySignatoryStatus::InviteAccepted { ts } => {
                CompanySignatoryStatusWeb::InviteAccepted { ts }
            }
            CompanySignatoryStatus::InviteRejected { ts } => {
                CompanySignatoryStatusWeb::InviteRejected { ts }
            }
            CompanySignatoryStatus::InviteAcceptedIdentityProven { ts, data, proof } => {
                CompanySignatoryStatusWeb::InviteAcceptedIdentityProven {
                    ts,
                    confirmation: (proof, data).into(),
                }
            }
            CompanySignatoryStatus::Removed { ts, remover } => {
                CompanySignatoryStatusWeb::Removed { ts, remover }
            }
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct CompanySignatoryWeb {
    #[tsify(type = "string")]
    pub node_id: NodeId,
    pub status: CompanySignatoryStatusWeb,
}

impl From<CompanySignatory> for CompanySignatoryWeb {
    fn from(value: CompanySignatory) -> Self {
        Self {
            node_id: value.node_id,
            status: value.status.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct CompanyKeysWeb {
    #[tsify(type = "string")]
    pub id: NodeId,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct CreateCompanyPayload {
    pub id: String,
    pub name: String,
    pub country_of_registration: Option<String>,
    pub city_of_registration: Option<String>,
    pub postal_address: CreatePostalAddressWeb,
    pub email: String,
    pub registration_number: Option<String>,
    pub registration_date: Option<String>,
    pub proof_of_registration_file_upload_id: Option<String>,
    pub logo_file_upload_id: Option<String>,
    pub creator_email: String,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct EditCompanyPayload {
    #[tsify(type = "string")]
    pub id: NodeId,
    pub name: Option<String>,
    pub email: Option<String>,
    pub postal_address: CreateOptionalPostalAddressWeb,
    pub country_of_registration: Option<String>,
    pub city_of_registration: Option<String>,
    pub registration_number: Option<String>,
    pub registration_date: Option<String>,
    pub logo_file_upload_id: Option<String>,
    pub proof_of_registration_file_upload_id: Option<String>,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct InviteSignatoryPayload {
    #[tsify(type = "string")]
    pub id: NodeId,
    #[tsify(type = "string")]
    pub signatory_node_id: NodeId,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct RemoveSignatoryPayload {
    #[tsify(type = "string")]
    pub id: NodeId,
    #[tsify(type = "string")]
    pub signatory_node_id: NodeId,
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct ListSignatoriesResponse {
    pub signatories: Vec<SignatoryResponse>,
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct SignatoryResponse {
    pub t: ContactTypeWeb,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string")]
    pub name: Name,
    pub postal_address: Option<PostalAddressWeb>,
    pub avatar_file: Option<FileWeb>,
    pub is_logical: bool,
    pub signatory: CompanySignatoryWeb,
}

impl TryFrom<(CompanySignatory, Contact)> for SignatoryResponse {
    type Error = ValidationError;

    fn try_from((signatory, contact): (CompanySignatory, Contact)) -> Result<Self, Self::Error> {
        if contact.t == ContactType::Anon {
            return Err(ValidationError::InvalidContact(contact.node_id.to_string()));
        }
        Ok(Self {
            t: contact.t.into(),
            node_id: contact.node_id.clone(),
            name: contact.name,
            postal_address: contact.postal_address.map(|pa| pa.into()),
            avatar_file: contact.avatar_file.map(|f| f.into()),
            is_logical: contact.is_logical,
            signatory: signatory.into(),
        })
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct ResyncCompanyPayload {
    #[tsify(type = "string")]
    pub node_id: NodeId,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ChangeSignatoryEmailPayload {
    pub id: String,
    pub email: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ConfirmEmailPayload {
    pub id: String,
    pub email: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct VerifyEmailPayload {
    pub id: String,
    pub confirmation_code: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct AcceptCompanyInvitePayload {
    pub id: String,
    pub email: String,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct LocallyHideSignatoryPayload {
    pub id: String,
    pub signatory_node_id: String,
}
