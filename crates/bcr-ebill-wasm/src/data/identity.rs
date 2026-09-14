use bcr_common::core::NodeId;
use bcr_ebill_api::service::{Error, Result};
use bcr_ebill_core::{
    application::{
        ValidationError,
        identity::{Identity, SwitchIdentityType},
        nostr_contact::NostrPublicKey,
    },
    protocol::{
        City, Country, Date, Email, EmailIdentityProofData, Identification, Name, PublicKey,
        SchnorrSignature, SignedIdentityProof, Timestamp, blockchain::identity::IdentityType,
    },
};
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use crate::data::CreateOptionalPostalAddressWeb;

use super::{FileWeb, OptionalPostalAddressWeb};

#[derive(Tsify, Debug, Serialize, Deserialize)]
pub struct SwitchIdentity {
    pub t: Option<SwitchIdentityTypeWeb>,
    #[tsify(type = "string")]
    pub node_id: NodeId,
}

#[wasm_bindgen]
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, serde_repr::Serialize_repr, serde_repr::Deserialize_repr, PartialEq, Eq,
)]
pub enum SwitchIdentityTypeWeb {
    Person = 0,
    Company = 1,
}

impl From<SwitchIdentityType> for SwitchIdentityTypeWeb {
    fn from(val: SwitchIdentityType) -> Self {
        match val {
            SwitchIdentityType::Person => SwitchIdentityTypeWeb::Person,
            SwitchIdentityType::Company => SwitchIdentityTypeWeb::Company,
        }
    }
}

#[wasm_bindgen]
#[repr(u8)]
#[derive(
    Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr, PartialEq, Eq,
)]
pub enum IdentityTypeWeb {
    Ident = 0,
    Anon = 1,
}

impl TryFrom<u64> for IdentityTypeWeb {
    type Error = Error;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        Ok(IdentityType::try_from(value)
            .map_err(|e| Self::Error::Validation(ValidationError::Protocol(e)))?
            .into())
    }
}

impl From<IdentityType> for IdentityTypeWeb {
    fn from(val: IdentityType) -> Self {
        match val {
            IdentityType::Ident => IdentityTypeWeb::Ident,
            IdentityType::Anon => IdentityTypeWeb::Anon,
        }
    }
}

impl From<IdentityTypeWeb> for IdentityType {
    fn from(value: IdentityTypeWeb) -> Self {
        match value {
            IdentityTypeWeb::Ident => IdentityType::Ident,
            IdentityTypeWeb::Anon => IdentityType::Anon,
        }
    }
}

#[derive(Tsify, Debug, Deserialize)]
pub struct NewIdentityPayload {
    pub t: u64,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: CreateOptionalPostalAddressWeb,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file_upload_id: Option<String>,
    pub identity_document_file_upload_id: Option<String>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ChangeIdentityPayload {
    pub name: Option<String>,
    pub postal_address: CreateOptionalPostalAddressWeb,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file_upload_id: Option<String>,
    pub identity_document_file_upload_id: Option<String>,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ConfirmEmailPayload {
    pub email: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct VerifyEmailPayload {
    pub confirmation_code: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct ChangeIdentityEmailPayload {
    pub email: String,
}

#[derive(Tsify, Debug, Serialize)]
pub struct IdentityWeb {
    pub t: IdentityTypeWeb,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string | undefined")]
    pub email: Option<Email>,
    #[tsify(type = "string")]
    pub bitcoin_public_key: PublicKey,
    #[tsify(type = "string")]
    pub npub: NostrPublicKey,
    pub postal_address: OptionalPostalAddressWeb,
    #[tsify(type = "string | undefined")]
    pub date_of_birth: Option<Date>,
    #[tsify(type = "string | undefined")]
    pub country_of_birth: Option<Country>,
    #[tsify(type = "string | undefined")]
    pub city_of_birth: Option<City>,
    #[tsify(type = "string | undefined")]
    pub identification_number: Option<Identification>,
    pub profile_picture_file: Option<FileWeb>,
    pub identity_document_file: Option<FileWeb>,
    #[tsify(type = "string[]")]
    pub nostr_relays: Vec<url::Url>,
}

impl IdentityWeb {
    pub fn from(identity: Identity) -> Result<Self> {
        Ok(Self {
            t: identity.t.into(),
            node_id: identity.node_id.clone(),
            name: identity.name,
            email: identity.email,
            bitcoin_public_key: identity.node_id.pub_key(),
            npub: identity.node_id.npub(),
            postal_address: identity.postal_address.into(),
            date_of_birth: identity.date_of_birth,
            country_of_birth: identity.country_of_birth,
            city_of_birth: identity.city_of_birth,
            identification_number: identity.identification_number,
            profile_picture_file: identity.profile_picture_file.map(|f| f.into()),
            identity_document_file: identity.identity_document_file.map(|f| f.into()),
            nostr_relays: identity.nostr_relays,
        })
    }
}

/// Response for a private key seeed backup
#[derive(Tsify, Debug, Serialize, Deserialize)]
pub struct SeedPhrase {
    /// The seed phrase of the current private key
    pub seed_phrase: String,
}

#[derive(Tsify, Debug, Serialize, Deserialize)]
pub struct ShareContactTo {
    /// The node id of the identity to share the contact details to
    #[tsify(type = "string")]
    pub recipient: NodeId,
}

#[derive(Tsify, Debug, Serialize, Deserialize)]
pub struct ShareCompanyContactTo {
    /// The node id of the identity to share the contact details to
    #[tsify(type = "string")]
    pub recipient: NodeId,

    /// The node id of the company to share the contact details for
    #[tsify(type = "string")]
    pub company_id: NodeId,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct IdentityEmailConfirmationWeb {
    #[tsify(type = "string")]
    pub signature: SchnorrSignature,
    #[tsify(type = "string")]
    pub witness: NodeId,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string | undefined")]
    pub company_node_id: Option<NodeId>,
    #[tsify(type = "string")]
    pub email: Email,
    #[tsify(type = "number")]
    pub created_at: Timestamp,
}

impl From<(SignedIdentityProof, EmailIdentityProofData)> for IdentityEmailConfirmationWeb {
    fn from((proof, data): (SignedIdentityProof, EmailIdentityProofData)) -> Self {
        Self {
            signature: proof.signature,
            witness: proof.witness,
            node_id: data.node_id,
            company_node_id: data.company_node_id,
            email: data.email,
            created_at: data.created_at,
        }
    }
}
