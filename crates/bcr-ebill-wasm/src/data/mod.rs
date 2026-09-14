use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_api::service::file_upload_service::{
    UploadFileHandler, detect_content_type_for_bytes,
};
use bcr_ebill_core::{
    application::{
        GeneralSearchFilterItemType, GeneralSearchResult, UploadFileResult, ValidationError,
    },
    protocol::{
        Address, City, Country, Date, EditOptionalFieldMode, File, Name, OptionalPostalAddress,
        PostalAddress, Sha256Hash, Timestamp, Zip,
    },
};
use bcr_ebill_persistence::notification::NotificationFilter;
use bill::LightBitcreditBillWeb;
use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use company::CompanyWeb;
use contact::ContactWeb;
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

pub mod bill;
pub mod company;
pub mod contact;
pub mod identity;
pub mod mint;
pub mod notification;

#[derive(Tsify, Debug, Serialize)]
pub struct StatusResponse {
    /// Name of the currently configured Bitcoin network (e.g. `mainnet`, `testnet`).
    pub bitcoin_network: String,
    /// `true` if the app has an active connection to at least one configured Nostr relay.
    ///
    /// This reflects the status of the Nostr transport layer, not general internet
    /// connectivity or backend/database availability. When `connected` is `false`,
    /// operations that require Nostr (such as bill synchronization with other peers,
    /// sending or receiving notifications, or other relay-based messaging) will not
    /// be able to communicate over the network and may fall back to local-only state.
    pub connected: bool,
    /// Semantic version of the running E-Bills application backend.
    pub app_version: String,
}

#[derive(Tsify, Debug, Serialize)]
pub struct GeneralSearchResponse {
    pub bills: Vec<LightBitcreditBillWeb>,
    pub contacts: Vec<ContactWeb>,
    pub companies: Vec<CompanyWeb>,
}

impl From<GeneralSearchResult> for GeneralSearchResponse {
    fn from(val: GeneralSearchResult) -> Self {
        GeneralSearchResponse {
            bills: val.bills.into_iter().map(|b| b.into()).collect(),
            contacts: val.contacts.into_iter().map(|c| c.into()).collect(),
            companies: val.companies.into_iter().map(|c| c.into()).collect(),
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct GeneralSearchFilterPayload {
    pub filter: GeneralSearchFilter,
}

#[derive(Tsify, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum GeneralSearchFilterItemTypeWeb {
    Company,
    Bill,
    Contact,
}

impl From<GeneralSearchFilterItemTypeWeb> for GeneralSearchFilterItemType {
    fn from(value: GeneralSearchFilterItemTypeWeb) -> Self {
        match value {
            GeneralSearchFilterItemTypeWeb::Company => GeneralSearchFilterItemType::Company,
            GeneralSearchFilterItemTypeWeb::Bill => GeneralSearchFilterItemType::Bill,
            GeneralSearchFilterItemTypeWeb::Contact => GeneralSearchFilterItemType::Contact,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSearchFilter {
    pub search_term: String,
    pub currency: String,
    pub item_types: Vec<GeneralSearchFilterItemTypeWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct OverviewResponse {
    pub currency: String,
    pub balances: OverviewBalanceResponse,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct OverviewBalanceResponse {
    pub payee: BalanceResponse,
    pub payer: BalanceResponse,
    pub contingent: BalanceResponse,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BalanceResponse {
    pub sum: String,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct CurrenciesResponse {
    pub currencies: Vec<CurrencyResponse>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct CurrencyResponse {
    pub code: String,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct CreateOptionalPostalAddressWeb {
    pub country: Option<String>,
    pub city: Option<String>,
    pub zip: Option<String>,
    pub address: Option<String>,
}

impl CreateOptionalPostalAddressWeb {
    pub fn is_none(&self) -> bool {
        self.country.is_none()
            && self.city.is_none()
            && self.zip.is_none()
            && self.address.is_none()
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct OptionalPostalAddressWeb {
    #[tsify(type = "string | undefined")]
    pub country: Option<Country>,
    #[tsify(type = "string | undefined")]
    pub city: Option<City>,
    #[tsify(type = "string | undefined")]
    pub zip: Option<Zip>,
    #[tsify(type = "string | undefined")]
    pub address: Option<Address>,
}

impl TryFrom<CreateOptionalPostalAddressWeb> for OptionalPostalAddressWeb {
    type Error = ValidationError;

    fn try_from(value: CreateOptionalPostalAddressWeb) -> Result<Self, Self::Error> {
        Ok(OptionalPostalAddressWeb {
            country: value.country.map(|c| Country::parse(&c)).transpose()?,
            city: value.city.map(City::new).transpose()?,
            zip: value.zip.map(Zip::new).transpose()?,
            address: value.address.map(Address::new).transpose()?,
        })
    }
}

impl OptionalPostalAddressWeb {
    pub fn is_none(&self) -> bool {
        self.country.is_none()
            && self.city.is_none()
            && self.zip.is_none()
            && self.address.is_none()
    }
}

impl From<OptionalPostalAddressWeb> for OptionalPostalAddress {
    fn from(value: OptionalPostalAddressWeb) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

impl From<OptionalPostalAddress> for OptionalPostalAddressWeb {
    fn from(value: OptionalPostalAddress) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct CreatePostalAddressWeb {
    pub country: String,
    pub city: String,
    pub zip: Option<String>,
    pub address: String,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct PostalAddressWeb {
    #[tsify(type = "string")]
    pub country: Country,
    #[tsify(type = "string")]
    pub city: City,
    #[tsify(type = "string | undefined")]
    pub zip: Option<Zip>,
    #[tsify(type = "string")]
    pub address: Address,
}

impl TryFrom<CreatePostalAddressWeb> for PostalAddressWeb {
    type Error = ValidationError;

    fn try_from(value: CreatePostalAddressWeb) -> Result<Self, Self::Error> {
        Ok(PostalAddressWeb {
            country: Country::parse(&value.country)?,
            city: City::new(&value.city)?,
            zip: value.zip.map(Zip::new).transpose()?,
            address: Address::new(&value.address)?,
        })
    }
}

impl From<PostalAddressWeb> for PostalAddress {
    fn from(value: PostalAddressWeb) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

impl From<PostalAddress> for PostalAddressWeb {
    fn from(val: PostalAddress) -> Self {
        PostalAddressWeb {
            country: val.country,
            city: val.city,
            zip: val.zip,
            address: val.address,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationFilters {
    pub active: Option<bool>,
    pub reference_id: Option<String>,
    pub notification_type: Option<String>,
    pub level: Option<String>,
    #[tsify(type = "string[] | undefined")]
    pub node_ids: Option<Vec<NodeId>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl From<NotificationFilters> for NotificationFilter {
    fn from(value: NotificationFilters) -> Self {
        Self {
            active: value.active,
            reference_id: value.reference_id,
            notification_type: value.notification_type,
            node_ids: value.node_ids.unwrap_or_default(),
            event_id: None,
            level: value.level,
            limit: value.limit,
            offset: value.offset,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize)]
pub struct FileWeb {
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string")]
    pub hash: Sha256Hash,
    #[tsify(type = "string")]
    pub nostr_hash: Sha256HexHash,
}

impl From<FileWeb> for File {
    fn from(value: FileWeb) -> Self {
        Self {
            name: value.name,
            hash: value.hash,
            nostr_hash: value.nostr_hash,
        }
    }
}

impl From<File> for FileWeb {
    fn from(val: File) -> Self {
        FileWeb {
            name: val.name,
            hash: val.hash,
            nostr_hash: val.nostr_hash,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize)]
pub struct BinaryFileResponse {
    pub data: Vec<u8>,
    #[tsify(type = "string")]
    pub name: Name,
    pub content_type: String,
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize)]
pub struct Base64FileResponse {
    pub data: String,
    #[tsify(type = "string")]
    pub name: Name,
    pub content_type: String,
}

#[derive(Tsify, Debug, Clone, Serialize, Deserialize)]
pub struct UploadFile {
    pub data: Vec<u8>,
    pub extension: Option<String>,
    pub name: String,
}

#[async_trait]
impl UploadFileHandler for UploadFile {
    async fn get_contents(&self) -> std::io::Result<Vec<u8>> {
        Ok(self.data.clone())
    }

    fn extension(&self) -> Option<String> {
        self.extension.clone()
    }

    fn name(&self) -> Option<String> {
        Some(self.name.clone())
    }

    fn len(&self) -> usize {
        self.data.len()
    }
    async fn detect_content_type(&self) -> std::io::Result<Option<String>> {
        Ok(detect_content_type_for_bytes(&self.data))
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct UploadFileResponse {
    #[tsify(type = "string")]
    pub file_upload_id: Uuid,
}

impl From<UploadFileResult> for UploadFileResponse {
    fn from(val: UploadFileResult) -> Self {
        UploadFileResponse {
            file_upload_id: val.file_upload_id,
        }
    }
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct BtcAddressPayload {
    pub address: String,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct BtcAddressAndSumPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    pub address: String,
    pub sum: String,
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct MempoolLinkResponse {
    pub mempool_link: String,
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LinkToPayResponse {
    pub link_to_pay: String,
}

// Checks if the given JS Value has the given field - also works with nested fields like postal_address.zip
fn has_field(js_value: &JsValue, field: &str) -> bool {
    let mut current = js_value.to_owned();

    for part in field.split('.') {
        let key = JsValue::from_str(part);

        if !js_sys::Reflect::has(&current, &key).unwrap_or(false) {
            return false;
        }

        match js_sys::Reflect::get(&current, &key) {
            Ok(v) => current = v,
            Err(_) => return false,
        }
    }

    true
}

// If the field is set with a value, set it, if it's set to undefined, unset it, if it's not there, ignore it
pub fn edit_field_mode<T>(
    js_value: &JsValue,
    field: &str,
    data: Option<T>,
) -> EditOptionalFieldMode<T> {
    if has_field(js_value, field) {
        match data {
            Some(d) => EditOptionalFieldMode::Set(d),
            None => EditOptionalFieldMode::Unset,
        }
    } else {
        EditOptionalFieldMode::Ignore
    }
}

/// Parses the given date of format YYYY-mm-dd to a UTC end-of-day timestamp
pub fn parse_deadline_string(deadline_date: &str) -> Result<Timestamp, ValidationError> {
    let ts = Date::new(deadline_date)?.to_timestamp().end_of_day();
    Ok(ts)
}
