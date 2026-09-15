use std::str::FromStr;

use super::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_api::service::{
    Error,
    bill_service::Error as BillServiceError,
    file_upload_service::{UploadFileHandler, detect_content_type_for_bytes},
    transport_service::ResyncMode,
};
use bcr_ebill_core::{
    application::{
        ValidationError,
        bill::{BillsFilterRole, LightBitcreditBillResult},
    },
    protocol::{
        City, Country, Currency, Date, Name, ProtocolValidationError, Sum, Timestamp,
        blockchain::{
            bill::{
                BillAction, BillIssueData, RecourseReason,
                participant::{BillAnonParticipant, BillIdentParticipant, BillParticipant},
            },
            identity::IdentityType,
        },
        crypto::BcrKeys,
    },
};
use log::error;
use uuid::Uuid;
use wasm_bindgen::prelude::*;

use crate::{
    TSResult,
    api::identity::get_current_identity_node_id,
    context::get_ctx,
    data::{
        Base64FileResponse, BinaryFileResponse, UploadFile, UploadFileResponse,
        bill::{
            AcceptBitcreditBillPayload, BillCheckSweepBTCFundsPayload, BillCombinedBitcoinKeyWeb,
            BillHistoryResponse, BillIdResponse, BillResetMintQuoteState, BillSweepBTCEstimateWeb,
            BillSweepBTCFundsPayload, BillSweepBTCFundsResultWeb, BillsResponse,
            BillsSearchFilterPayload, BitcreditBillPayload, BitcreditBillWeb,
            EndorseBitcreditBillPayload, EndorsementsResponse, LightBillsResponse,
            OfferToSellBitcreditBillPayload, OverrideBillFromNostrPayload, PastEndorseesResponse,
            PastPaymentsResponse, RejectActionBillPayload, RequestRecourseForAcceptancePayload,
            RequestRecourseForPaymentPayload, RequestToAcceptBitcreditBillPayload,
            RequestToMintBitcreditBillPayload, RequestToPayAsMintBitcreditBillPayload,
            RequestToPayBitcreditBillPayload, ResyncBillPayload, ShareBillWithCourtPayload,
        },
        mint::MintRequestStateResponse,
        parse_deadline_string,
    },
    error::WasmError,
};

use super::identity::get_current_identity;

async fn get_attachment(bill_id: &str, file_name: &Name) -> Result<(Vec<u8>, String)> {
    let parsed_bill_id = BillId::from_str(bill_id).map_err(ProtocolValidationError::from)?;
    let current_timestamp = Timestamp::now();
    let identity = get_ctx().identity_service.get_identity().await?;
    let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
    // get bill
    let bill = get_ctx()
        .bill_service
        .get_detail(
            &parsed_bill_id,
            &identity,
            &caller_public_data,
            &caller_keys,
            current_timestamp,
        )
        .await?;

    // check if this file even exists on the bill
    let file = match bill.data.files.iter().find(|f| &f.name == file_name) {
        Some(f) => f,
        None => {
            return Err(bcr_ebill_api::service::bill_service::Error::NotFound.into());
        }
    };

    // fetch the attachment
    let keys = get_ctx()
        .bill_service
        .get_bill_keys(&parsed_bill_id)
        .await?;
    let file_bytes = get_ctx()
        .bill_service
        .open_and_decrypt_attached_file(&parsed_bill_id, file, &keys.get_private_key())
        .await?;

    let content_type = detect_content_type_for_bytes(&file_bytes).ok_or(Error::Validation(
        ProtocolValidationError::InvalidContentType.into(),
    ))?;
    Ok((file_bytes, content_type))
}

#[wasm_bindgen]
pub struct Bill;

#[wasm_bindgen]
impl Bill {
    #[wasm_bindgen]
    pub fn new() -> Self {
        Bill
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<EndorsementsResponse>")]
    pub async fn endorsements(&self, id: &str) -> JsValue {
        let res: Result<EndorsementsResponse> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let current_timestamp = Timestamp::now();
            let identity = get_ctx().identity_service.get_identity().await?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let result = get_ctx()
                .bill_service
                .get_endorsements(
                    &bill_id,
                    &identity,
                    &caller_public_data,
                    &caller_keys,
                    current_timestamp,
                )
                .await?;
            Ok(EndorsementsResponse {
                endorsements: result.into_iter().map(|e| e.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<PastPaymentsResponse>")]
    pub async fn past_payments(&self, id: &str) -> JsValue {
        let res: Result<PastPaymentsResponse> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let result = get_ctx()
                .bill_service
                .get_past_payments(
                    &bill_id,
                    &caller_public_data,
                    &caller_keys,
                    Timestamp::now(),
                )
                .await?;
            Ok(PastPaymentsResponse {
                past_payments: result.into_iter().map(|e| e.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<PastEndorseesResponse>")]
    pub async fn past_endorsees(&self, id: &str) -> JsValue {
        let res: Result<PastEndorseesResponse> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let result = get_ctx()
                .bill_service
                .get_past_endorsees(&bill_id, &get_current_identity_node_id().await?)
                .await?;
            Ok(PastEndorseesResponse {
                past_endorsees: result.into_iter().map(|e| e.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillCombinedBitcoinKeyWeb[]>")]
    pub async fn bitcoin_keys(&self, id: &str) -> JsValue {
        let res: Result<Vec<BillCombinedBitcoinKeyWeb>> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let combined_keys = get_ctx()
                .bill_service
                .get_combined_bitcoin_keys_for_bill(&bill_id, &caller_public_data, &caller_keys)
                .await?;
            Ok(combined_keys.into_iter().map(|ck| ck.into()).collect())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BinaryFileResponse>")]
    pub async fn attachment(&self, bill_id: &str, file_name: &str) -> JsValue {
        let res: Result<BinaryFileResponse> = async {
            let name = Name::new(file_name)?;
            let (file_bytes, content_type) = get_attachment(bill_id, &name).await?;
            Ok(BinaryFileResponse {
                data: file_bytes,
                name,
                content_type,
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<Base64FileResponse>")]
    pub async fn attachment_base64(&self, bill_id: &str, file_name: &str) -> JsValue {
        let res: Result<Base64FileResponse> = async {
            let name = Name::new(file_name)?;
            let (file_bytes, content_type) = get_attachment(bill_id, &name).await?;
            Ok(Base64FileResponse {
                data: STANDARD.encode(&file_bytes),
                name,
                content_type,
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<UploadFileResponse>")]
    pub async fn upload(
        &self,
        #[wasm_bindgen(unchecked_param_type = "UploadFile")] payload: JsValue,
    ) -> JsValue {
        let res: Result<UploadFileResponse> = async {
            let upload_file: UploadFile = serde_wasm_bindgen::from_value(payload)?;
            let upload_file_handler: &dyn UploadFileHandler =
                &upload_file as &dyn UploadFileHandler;

            get_ctx()
                .file_upload_service
                .validate_attached_file(upload_file_handler)
                .await?;

            let file_upload_response = get_ctx()
                .file_upload_service
                .upload_file(upload_file_handler)
                .await?;

            Ok(file_upload_response.into())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<LightBillsResponse>")]
    pub async fn search(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BillsSearchFilterPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<LightBillsResponse> = async {
            let filter_payload: BillsSearchFilterPayload = serde_wasm_bindgen::from_value(payload)?;
            let filter = filter_payload.filter;

            let (from, to) = match filter.date_range {
                None => (None, None),
                Some(date_range) => {
                    let from = Date::new(&date_range.from)?.to_timestamp();
                    // Change the date to the end of the day, so we collect bills during the day as well
                    let to = Date::new(&date_range.to)
                        .map(|d| d.to_timestamp())
                        .map(|ts| ts.end_of_day())?;
                    (Some(from), Some(to))
                }
            };
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let bills = get_ctx()
                .bill_service
                .search_bills(
                    &Currency::sat(),
                    &filter.search_term,
                    from,
                    to,
                    &BillsFilterRole::from(filter.role),
                    &filter.participants,
                    &caller_public_data,
                    &caller_keys,
                )
                .await?;

            Ok(LightBillsResponse {
                bills: bills.into_iter().map(|b| b.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<LightBillsResponse>")]
    pub async fn list_light(&self) -> JsValue {
        let res: Result<LightBillsResponse> = async {
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let bills: Vec<LightBitcreditBillResult> = get_ctx()
                .bill_service
                .get_bills(&caller_public_data, &caller_keys)
                .await?
                .into_iter()
                .map(|b| b.into())
                .collect();
            Ok(LightBillsResponse {
                bills: bills.into_iter().map(|b| b.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillsResponse>")]
    pub async fn list(&self) -> JsValue {
        let res: Result<BillsResponse> = async {
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let bills = get_ctx()
                .bill_service
                .get_bills(&caller_public_data, &caller_keys)
                .await?;
            Ok(BillsResponse {
                bills: bills.into_iter().map(|b| b.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BitcreditBillWeb>")]
    pub async fn detail(&self, id: &str) -> JsValue {
        let res: Result<BitcreditBillWeb> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let current_timestamp = Timestamp::now();
            let identity = get_ctx().identity_service.get_identity().await?;

            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let bill_detail = get_ctx()
                .bill_service
                .get_detail(
                    &bill_id,
                    &identity,
                    &caller_public_data,
                    &caller_keys,
                    current_timestamp,
                )
                .await?;

            Ok(bill_detail.into())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn check_payment_for_bill(&self, id: &str) -> JsValue {
        let res: Result<()> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let identity = get_ctx().identity_service.get_full_identity().await?;
            if let Err(e) = get_ctx()
                .bill_service
                .check_payment_for_bill(&bill_id, &identity.identity)
                .await
            {
                error!("Error while checking bill payment for {id}: {e}");
            }

            if let Err(e) = get_ctx()
                .bill_service
                .check_offer_to_sell_payment_for_bill(&bill_id, &identity)
                .await
            {
                error!("Error while checking bill offer to sell payment for {id}: {e}");
            }

            if let Err(e) = get_ctx()
                .bill_service
                .check_recourse_payment_for_bill(&bill_id, &identity)
                .await
            {
                error!("Error while checking bill recourse payment for {id}: {e}");
            }
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn check_payment(&self) -> JsValue {
        let res: Result<()> = async {
            if let Err(e) = get_ctx().bill_service.check_bills_payment().await {
                error!("Error while checking bills payment: {e}");
            }

            if let Err(e) = get_ctx()
                .bill_service
                .check_bills_offer_to_sell_payment()
                .await
            {
                error!("Error while checking bills offer to sell payment: {e}");
            }

            if let Err(e) = get_ctx()
                .bill_service
                .check_bills_in_recourse_payment()
                .await
            {
                error!("Error while checking bills recourse payment: {e}");
            }
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    async fn issue_bill(
        &self,
        bill_payload: BitcreditBillPayload,
        timestamp: Timestamp,
        blank_issue: bool,
    ) -> Result<BillId> {
        let (drawer_public_data, drawer_keys) = get_signer_public_data_and_keys().await?;

        let mut parsed_file_upload_ids: Vec<Uuid> =
            Vec::with_capacity(bill_payload.file_upload_ids.len());

        for file_upload_id in bill_payload.file_upload_ids.iter() {
            parsed_file_upload_ids.push(
                Uuid::from_str(file_upload_id)
                    .map_err(|_| ProtocolValidationError::InvalidFileUploadId)?,
            );
        }

        let bill = get_ctx()
            .bill_service
            .issue_new_bill(BillIssueData {
                t: bill_payload.t,
                country_of_issuing: Country::parse(&bill_payload.country_of_issuing)?,
                city_of_issuing: City::new(bill_payload.city_of_issuing)?,
                issue_date: Date::new(bill_payload.issue_date)?,
                maturity_date: Date::new(bill_payload.maturity_date)?,
                drawee: NodeId::from_str(&bill_payload.drawee)
                    .map_err(ProtocolValidationError::from)?,
                payee: NodeId::from_str(&bill_payload.payee)
                    .map_err(ProtocolValidationError::from)?,
                sum: Sum::new_sat_from_str(&bill_payload.sum)?,
                country_of_payment: Country::parse(&bill_payload.country_of_payment)?,
                city_of_payment: City::new(bill_payload.city_of_payment)?,
                file_upload_ids: parsed_file_upload_ids,
                drawer_public_data: drawer_public_data.clone(),
                drawer_keys: drawer_keys.clone(),
                timestamp,
                blank_issue,
            })
            .await?;

        Ok(bill.id)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillIdResponse>")]
    pub async fn issue(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<BillIdResponse> = async {
            let bill_payload: BitcreditBillPayload = serde_wasm_bindgen::from_value(payload)?;
            let timestamp = Timestamp::now();
            let bill_id = self.issue_bill(bill_payload, timestamp, false).await?;
            Ok(BillIdResponse { id: bill_id })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillIdResponse>")]
    pub async fn issue_blank(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<BillIdResponse> = async {
            let bill_payload: BitcreditBillPayload = serde_wasm_bindgen::from_value(payload)?;
            let timestamp = Timestamp::now();
            let bill_id = self.issue_bill(bill_payload, timestamp, true).await?;
            Ok(BillIdResponse { id: bill_id })
        }
        .await;
        TSResult::res_to_js(res)
    }

    async fn offer_to_sell_bill(
        &self,
        payload: OfferToSellBitcreditBillPayload,
        buyer: BillParticipant,
        timestamp: Timestamp,
        sum: Sum,
        buying_deadline_timestamp: Timestamp,
    ) -> Result<()> {
        let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

        get_ctx()
            .bill_service
            .execute_bill_action(
                &payload.bill_id,
                BillAction::OfferToSell(buyer.clone(), sum, buying_deadline_timestamp),
                &signer_public_data,
                &signer_keys,
                timestamp,
            )
            .await?;

        Ok(())
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn offer_to_sell(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OfferToSellBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let offer_to_sell_payload: OfferToSellBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let public_data_buyer = match get_ctx()
                .contact_service
                .get_identity_by_node_id(&offer_to_sell_payload.buyer)
                .await
            {
                Ok(Some(buyer)) => buyer,
                Ok(None) | Err(_) => {
                    return Err(
                        BillServiceError::Validation(ValidationError::BuyerNotInContacts).into(),
                    );
                }
            };

            let sum = Sum::new_sat_from_str(&offer_to_sell_payload.sum)?;
            let timestamp = Timestamp::now();
            let deadline_ts = Date::new(&offer_to_sell_payload.buying_deadline)?.to_timestamp();
            self.offer_to_sell_bill(
                offer_to_sell_payload,
                public_data_buyer,
                timestamp,
                sum,
                deadline_ts,
            )
            .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    /// Blank offer to sell - the contact doesn't have to be an anonymous contact
    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn offer_to_sell_blank(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OfferToSellBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let offer_to_sell_payload: OfferToSellBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let public_data_buyer: BillAnonParticipant = match get_ctx()
                .contact_service
                .get_identity_by_node_id(&offer_to_sell_payload.buyer)
                .await
            {
                Ok(Some(buyer)) => buyer.into(), // turn contact into anonymous participant
                Ok(None) | Err(_) => {
                    return Err(
                        BillServiceError::Validation(ValidationError::BuyerNotInContacts).into(),
                    );
                }
            };

            let sum = Sum::new_sat_from_str(&offer_to_sell_payload.sum)?;
            let timestamp = Timestamp::now();
            let deadline_ts = parse_deadline_string(&offer_to_sell_payload.buying_deadline)?;
            self.offer_to_sell_bill(
                offer_to_sell_payload,
                BillParticipant::Anon(public_data_buyer),
                timestamp,
                sum,
                deadline_ts,
            )
            .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    async fn endorse(
        &self,
        payload: EndorseBitcreditBillPayload,
        endorsee: BillParticipant,
        timestamp: Timestamp,
    ) -> Result<()> {
        let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;
        get_ctx()
            .bill_service
            .execute_bill_action(
                &payload.bill_id,
                BillAction::Endorse(endorsee),
                &signer_public_data,
                &signer_keys,
                timestamp,
            )
            .await?;
        Ok(())
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn endorse_bill(
        &self,
        #[wasm_bindgen(unchecked_param_type = "EndorseBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let endorse_bill_payload: EndorseBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let public_data_endorsee = match get_ctx()
                .contact_service
                .get_identity_by_node_id(
                    &NodeId::from_str(&endorse_bill_payload.endorsee)
                        .map_err(ProtocolValidationError::from)?,
                )
                .await
            {
                Ok(Some(endorsee)) => endorsee,
                Ok(None) | Err(_) => {
                    return Err(BillServiceError::Validation(
                        ValidationError::EndorseeNotInContacts,
                    )
                    .into());
                }
            };
            let timestamp = Timestamp::now();
            self.endorse(endorse_bill_payload, public_data_endorsee, timestamp)
                .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    /// Blank endorsement - the contact doesn't have to be an anonymous contact
    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn endorse_bill_blank(
        &self,
        #[wasm_bindgen(unchecked_param_type = "EndorseBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let endorse_bill_payload: EndorseBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let public_data_endorsee_blank: BillAnonParticipant = match get_ctx()
                .contact_service
                .get_identity_by_node_id(
                    &NodeId::from_str(&endorse_bill_payload.endorsee)
                        .map_err(ProtocolValidationError::from)?,
                )
                .await
            {
                Ok(Some(endorsee)) => endorsee.into(), // turn contact into anonymous participant
                Ok(None) | Err(_) => {
                    return Err(BillServiceError::Validation(
                        ValidationError::EndorseeNotInContacts,
                    )
                    .into());
                }
            };
            let timestamp = Timestamp::now();
            self.endorse(
                endorse_bill_payload,
                BillParticipant::Anon(public_data_endorsee_blank),
                timestamp,
            )
            .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_pay(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestToPayBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_to_pay_bill_payload: RequestToPayBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &request_to_pay_bill_payload.bill_id,
                    BillAction::RequestToPay(
                        Currency::sat(), // TODO (currency): parse and use given currency
                        parse_deadline_string(&request_to_pay_bill_payload.payment_deadline)?,
                        None,
                    ),
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_pay_as_mint(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestToPayAsMintBitcreditBillPayload")]
        payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_to_pay_bill_payload: RequestToPayAsMintBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &request_to_pay_bill_payload.bill_id,
                    BillAction::RequestToPay(
                        Currency::sat(), // TODO (currency): parse and use given currency
                        parse_deadline_string(&request_to_pay_bill_payload.payment_deadline)?,
                        Some(request_to_pay_bill_payload.payment_address),
                    ),
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_accept(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestToAcceptBitcreditBillPayload")]
        payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_to_accept_bill_payload: RequestToAcceptBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &request_to_accept_bill_payload.bill_id,
                    BillAction::RequestAcceptance(parse_deadline_string(
                        &request_to_accept_bill_payload.acceptance_deadline,
                    )?),
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn accept(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AcceptBitcreditBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let accept_bill_payload: AcceptBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &accept_bill_payload.bill_id,
                    BillAction::Accept,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_mint(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestToMintBitcreditBillPayload")]
        payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_to_mint_bill_payload: RequestToMintBitcreditBillPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;
            get_ctx()
                .bill_service
                .request_to_mint(
                    &request_to_mint_bill_payload.bill_id,
                    &NodeId::from_str(&request_to_mint_bill_payload.mint_node)
                        .map_err(ProtocolValidationError::from)?,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<MintRequestStateResponse>")]
    pub async fn mint_state(&self, id: &str) -> JsValue {
        let res: Result<MintRequestStateResponse> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            let result = get_ctx()
                .bill_service
                .get_mint_state(&bill_id, &get_current_identity_node_id().await?)
                .await?;
            Ok(MintRequestStateResponse {
                request_states: result.into_iter().map(|e| e.into()).collect(),
            })
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn check_mint_state(&self, id: &str) -> JsValue {
        let res: Result<()> = async {
            let bill_id = BillId::from_str(id).map_err(ProtocolValidationError::from)?;
            get_ctx()
                .bill_service
                .check_mint_state(&bill_id, &get_current_identity_node_id().await?)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn cancel_request_to_mint(&self, mint_request_id: &str) -> JsValue {
        let res: Result<()> = async {
            let parsed_id = Uuid::from_str(mint_request_id)
                .map_err(|_| ProtocolValidationError::InvalidMintRequestId)?;
            get_ctx()
                .bill_service
                .cancel_request_to_mint(&parsed_id, &get_current_identity_node_id().await?)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn accept_mint_offer(&self, mint_request_id: &str) -> JsValue {
        let res: Result<()> = async {
            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;
            let parsed_id = Uuid::from_str(mint_request_id)
                .map_err(|_| ProtocolValidationError::InvalidMintRequestId)?;
            get_ctx()
                .bill_service
                .accept_mint_offer(&parsed_id, &signer_public_data, &signer_keys, timestamp)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn reject_mint_offer(&self, mint_request_id: &str) -> JsValue {
        let res: Result<()> = async {
            let parsed_id = Uuid::from_str(mint_request_id)
                .map_err(|_| ProtocolValidationError::InvalidMintRequestId)?;
            get_ctx()
                .bill_service
                .reject_mint_offer(&parsed_id, &get_current_identity_node_id().await?)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn reject_to_accept(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RejectActionBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let reject_payload: RejectActionBillPayload = serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &reject_payload.bill_id,
                    BillAction::RejectAcceptance,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn reject_to_pay(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RejectActionBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let reject_payload: RejectActionBillPayload = serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &reject_payload.bill_id,
                    BillAction::RejectPayment,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn reject_to_buy(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RejectActionBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let reject_payload: RejectActionBillPayload = serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &reject_payload.bill_id,
                    BillAction::RejectBuying,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn reject_to_pay_recourse(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RejectActionBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let reject_payload: RejectActionBillPayload = serde_wasm_bindgen::from_value(payload)?;

            let timestamp = Timestamp::now();
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

            get_ctx()
                .bill_service
                .execute_bill_action(
                    &reject_payload.bill_id,
                    BillAction::RejectPaymentForRecourse,
                    &signer_public_data,
                    &signer_keys,
                    timestamp,
                )
                .await?;

            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_recourse_bill_payment(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestRecourseForPaymentPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_recourse_payload: RequestRecourseForPaymentPayload =
                serde_wasm_bindgen::from_value(payload)?;
            let sum = Sum::new_sat_from_str(&request_recourse_payload.sum)?;

            request_recourse(
                RecourseReason::Pay(sum),
                &request_recourse_payload.bill_id,
                &request_recourse_payload.recoursee,
                parse_deadline_string(&request_recourse_payload.recourse_deadline)?,
            )
            .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn request_to_recourse_bill_acceptance(
        &self,
        #[wasm_bindgen(unchecked_param_type = "RequestRecourseForPaymentPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let request_recourse_payload: RequestRecourseForAcceptancePayload =
                serde_wasm_bindgen::from_value(payload)?;

            request_recourse(
                RecourseReason::Accept,
                &request_recourse_payload.bill_id,
                &request_recourse_payload.recoursee,
                parse_deadline_string(&request_recourse_payload.recourse_deadline)?,
            )
            .await
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn clear_bill_cache(&self) -> JsValue {
        let res: Result<()> = async {
            get_ctx().bill_service.clear_bill_cache().await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    /// Given a bill id, resync the chain via block transport
    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn sync_bill_chain(
        &self,
        #[wasm_bindgen(unchecked_param_type = "ResyncBillPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let payload: ResyncBillPayload = serde_wasm_bindgen::from_value(payload)?;
            get_ctx()
                .transport_service
                .block_transport()
                .resync_bill_chain(
                    &payload.bill_id,
                    payload.from_nostr.unwrap_or(false),
                    ResyncMode::Normal,
                )
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    /// Given a bill id, override the bill chain with the state from nostr
    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn dev_mode_override_bill_chain_from_nostr(
        &self,
        #[wasm_bindgen(unchecked_param_type = "OverrideBillFromNostrPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let payload: OverrideBillFromNostrPayload = serde_wasm_bindgen::from_value(payload)?;
            get_ctx()
                .transport_service
                .block_transport()
                .resync_bill_chain(&payload.bill_id, true, ResyncMode::NostrAuthoritative)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    /// Given a bill id, reset the local mint quote state for this bill
    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn dev_mode_reset_bill_mint_quote_state(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BillResetMintQuoteState")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let payload: BillResetMintQuoteState = serde_wasm_bindgen::from_value(payload)?;
            get_ctx()
                .bill_service
                .dev_mode_reset_bill_mint_quote_state(&payload.bill_id)
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<string[]>")]
    pub async fn dev_mode_get_full_bill_chain(&self, bill_id: &str) -> JsValue {
        let res: Result<Vec<String>> = async {
            let parsed_bill_id =
                BillId::from_str(bill_id).map_err(ProtocolValidationError::from)?;
            let plaintext_chain = get_ctx()
                .bill_service
                .dev_mode_get_full_bill_chain(
                    &parsed_bill_id,
                    &get_current_identity_node_id().await?,
                )
                .await?;
            let json_string_chain: Result<Vec<String>> = plaintext_chain
                .into_iter()
                .map(|plaintext_block| {
                    plaintext_block
                        .to_json_text()
                        .map_err(|e| WasmError::Service(Error::Protocol(e.into())))
                })
                .collect();

            json_string_chain
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<void>")]
    pub async fn share_bill_with_court(
        &self,
        #[wasm_bindgen(unchecked_param_type = "ShareBillWithCourtPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<()> = async {
            let payload: ShareBillWithCourtPayload = serde_wasm_bindgen::from_value(payload)?;
            let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;
            get_ctx()
                .bill_service
                .share_bill_with_court(
                    &payload.bill_id,
                    &signer_public_data,
                    &signer_keys,
                    &payload.court_node_id,
                )
                .await?;
            Ok(())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillHistoryResponse>")]
    pub async fn bill_history(&self, bill_id: &str) -> JsValue {
        let res: Result<BillHistoryResponse> = async {
            let parsed_bill_id =
                BillId::from_str(bill_id).map_err(ProtocolValidationError::from)?;
            let current_timestamp = Timestamp::now();
            let identity = get_ctx().identity_service.get_identity().await?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let res: BillHistoryResponse = get_ctx()
                .bill_service
                .get_bill_history(
                    &parsed_bill_id,
                    &identity,
                    &caller_public_data,
                    &caller_keys,
                    current_timestamp,
                )
                .await?
                .into();
            Ok(res)
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillSweepBTCEstimateWeb>")]
    pub async fn check_and_estimate_btc_sweep(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BillCheckSweepBTCFundsPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<BillSweepBTCEstimateWeb> = async {
            let payload: BillCheckSweepBTCFundsPayload = serde_wasm_bindgen::from_value(payload)?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let estimate = get_ctx()
                .bill_service
                .check_and_estimate_btc_sweep(
                    &payload.bill_id,
                    &caller_public_data,
                    &caller_keys,
                    &payload.source_address,
                    &payload.destination_address,
                )
                .await?;
            Ok(estimate.into())
        }
        .await;
        TSResult::res_to_js(res)
    }

    #[wasm_bindgen(unchecked_return_type = "TSResult<BillSweepBTCFundsResultWeb>")]
    pub async fn sweep_btc_funds(
        &self,
        #[wasm_bindgen(unchecked_param_type = "BillSweepBTCFundsPayload")] payload: JsValue,
    ) -> JsValue {
        let res: Result<BillSweepBTCFundsResultWeb> = async {
            let payload: BillSweepBTCFundsPayload = serde_wasm_bindgen::from_value(payload)?;
            let (caller_public_data, caller_keys) = get_signer_public_data_and_keys().await?;
            let res = get_ctx()
                .bill_service
                .btc_sweep(
                    &payload.bill_id,
                    &caller_public_data,
                    &caller_keys,
                    &payload.source_address,
                    &payload.destination_address,
                    payload.fee,
                )
                .await?;
            Ok(res.into())
        }
        .await;
        TSResult::res_to_js(res)
    }
}

async fn request_recourse(
    recourse_reason: RecourseReason,
    bill_id: &BillId,
    recoursee_node_id: &NodeId,
    recourse_deadline_timestamp: Timestamp,
) -> Result<()> {
    let timestamp = Timestamp::now();
    let (signer_public_data, signer_keys) = get_signer_public_data_and_keys().await?;

    // we fetch the nostr contact first to know where we have to send
    let nostr_contact = match get_ctx()
        .contact_service
        .get_nostr_contact_by_node_id(recoursee_node_id)
        .await
    {
        Ok(Some(nc)) => nc,
        Ok(None) | Err(_) => {
            return Err(
                BillServiceError::Validation(ValidationError::RecourseeNotInContacts).into(),
            );
        }
    };

    // fetch past endorsees to validate the recoursee is in there and to get their data
    let past_endorsees = get_ctx()
        .bill_service
        .get_past_endorsees(bill_id, &get_current_identity_node_id().await?)
        .await?;

    // create public recourse data from past endorsees and our nostr contacts
    let mut public_data_recoursee = match past_endorsees
        .iter()
        .find(|pe| &pe.pay_to_the_order_of.node_id == recoursee_node_id)
    {
        Some(found_pe) => found_pe.pay_to_the_order_of.clone(),
        None => {
            return Err(BillServiceError::Validation(
                ProtocolValidationError::RecourseeNotPastHolder.into(),
            )
            .into());
        }
    };
    public_data_recoursee.nostr_relays = nostr_contact.relays;

    get_ctx()
        .bill_service
        .execute_bill_action(
            bill_id,
            BillAction::RequestRecourse(
                public_data_recoursee,
                recourse_reason,
                recourse_deadline_timestamp,
            ),
            &signer_public_data,
            &signer_keys,
            timestamp,
        )
        .await?;

    Ok(())
}

impl Default for Bill {
    fn default() -> Self {
        Bill
    }
}

pub(super) async fn get_signer_public_data_and_keys() -> Result<(BillParticipant, BcrKeys)> {
    let current_identity = get_current_identity().await?;
    let local_node_id = current_identity.personal;
    let identity = get_ctx().identity_service.get_full_identity().await?;
    let (signer_public_data, signer_keys) = match current_identity.company {
        None => {
            match identity.identity.t {
                IdentityType::Ident => {
                    match BillIdentParticipant::new(identity.identity) {
                        Ok(identity_public_data) => (
                            BillParticipant::Ident(identity_public_data),
                            identity.key_pair,
                        ),
                        Err(_) => {
                            // only non-anon bill issuers can sign a bill
                            return Err(Error::Validation(
                                ProtocolValidationError::SignerCantBeAnon.into(),
                            )
                            .into());
                        }
                    }
                }
                IdentityType::Anon => (
                    BillParticipant::Anon(BillAnonParticipant::new(identity.identity)),
                    identity.key_pair,
                ),
            }
        }
        Some(company_node_id) => {
            let (company, keys) = get_ctx()
                .company_service
                .get_company_and_keys_by_id(&company_node_id)
                .await?;
            if !company
                .signatories
                .iter()
                .any(|s| s.node_id == local_node_id)
            {
                return Err(Error::Validation(
                    ProtocolValidationError::NotASignatory(local_node_id.to_string()).into(),
                )
                .into());
            }
            let mut as_ident = BillIdentParticipant::from(company);
            // use nostr relays from personal identity
            as_ident.nostr_relays = identity.identity.nostr_relays;
            (
                BillParticipant::Ident(as_ident),
                BcrKeys::from_private_key(&keys.get_private_key()),
            )
        }
    };
    Ok((signer_public_data, signer_keys))
}
