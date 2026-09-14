use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::{
    application::{
        bill::{
            BillAcceptState, BillAcceptanceStatus, BillCallerActions, BillCallerBillAction,
            BillCallerPayment, BillCallerPaymentAction, BillCallerPaymentState,
            BillCombinedBitcoinKey, BillCurrentWaitingState, BillData, BillMintState,
            BillMintStatus, BillParticipants, BillPaymentState, BillPaymentStatus,
            BillRecourseStatus, BillSellStatus, BillState, BillStatus, BillWaitingForPaymentState,
            BillWaitingForRecourseState, BillWaitingForSellState, BillWaitingStatePaymentData,
            BillsFilterRole, BitcreditBillResult, Endorsement, LightBitcreditBillResult,
            LightSignedBy, PastPaymentDataPayment, PastPaymentDataRecourse, PastPaymentDataSell,
            PastPaymentResult, SweepEstimate, SweepOption, SweepResult,
        },
        contact::{
            LightBillAnonParticipant, LightBillIdentParticipant,
            LightBillIdentParticipantWithAddress, LightBillParticipant, LightBillSignatory,
        },
    },
    protocol::{
        BitcoinAddress, BlockId, City, Country, Date, Email, Name, Timestamp,
        blockchain::bill::{
            BillHistory, BillHistoryBlock, BillHistoryBlockPaymentData, BillOpCode, PaymentStatus,
            participant::{
                BillAnonParticipant, BillIdentParticipant, BillParticipant, PastEndorsee, SignedBy,
            },
        },
        crypto::btc::BtcDescriptor,
    },
};

use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use super::{FileWeb, PostalAddressWeb, contact::ContactTypeWeb, notification::NotificationWeb};

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillIdResponse {
    #[tsify(type = "string")]
    pub id: BillId,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct BitcreditBillPayload {
    pub t: u64,
    pub country_of_issuing: String,
    pub city_of_issuing: String,
    pub issue_date: String,
    pub maturity_date: String,
    pub payee: String,
    pub drawee: String,
    pub sum: String,
    #[allow(unused)]
    pub currency: String,
    pub country_of_payment: String,
    pub city_of_payment: String,
    pub file_upload_ids: Vec<String>,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct EndorseBitcreditBillPayload {
    pub endorsee: String,
    #[tsify(type = "string")]
    pub bill_id: BillId,
}

#[derive(Tsify, Debug, Deserialize, Clone)]
pub struct RequestToMintBitcreditBillPayload {
    pub mint_node: String,
    #[tsify(type = "string")]
    pub bill_id: BillId,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct OfferToSellBitcreditBillPayload {
    #[tsify(type = "string")]
    pub buyer: NodeId,
    #[tsify(type = "string")]
    pub bill_id: BillId,
    pub sum: String,
    #[allow(unused)]
    pub currency: String,
    pub buying_deadline: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[allow(unused)]
    pub currency: String,
    pub payment_deadline: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct RequestToPayAsMintBitcreditBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[allow(unused)]
    pub currency: String,
    pub payment_deadline: String,
    #[tsify(type = "string")]
    pub payment_address: BitcoinAddress,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct RequestRecourseForPaymentPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[tsify(type = "string")]
    pub recoursee: NodeId,
    #[allow(unused)]
    pub currency: String,
    pub sum: String,
    pub recourse_deadline: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct RequestRecourseForAcceptancePayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[tsify(type = "string")]
    pub recoursee: NodeId,
    pub recourse_deadline: String,
}

#[derive(Tsify, Debug, Deserialize)]
pub struct AcceptBitcreditBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct RequestToAcceptBitcreditBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    pub acceptance_deadline: String,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct RejectActionBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillCombinedBitcoinKeyWeb {
    #[tsify(type = "number")]
    pub block_id: BlockId,
    #[tsify(type = "number")]
    pub signing_timestamp: Timestamp,
    pub payment_op: BillOpCodeWeb,
    #[tsify(type = "string")]
    pub private_descriptor: BtcDescriptor,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct BillCheckSweepBTCFundsPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[tsify(type = "string")]
    pub source_address: BitcoinAddress,
    #[tsify(type = "string")]
    pub destination_address: BitcoinAddress,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillSweepBTCEstimateWeb {
    pub available_funds: u64,
    pub economy: BillSweepBTCOptionWeb,
    pub fast: BillSweepBTCOptionWeb,
}

impl From<SweepEstimate> for BillSweepBTCEstimateWeb {
    fn from(val: SweepEstimate) -> Self {
        Self {
            available_funds: val.available_funds,
            economy: val.economy.into(),
            fast: val.fast.into(),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillSweepBTCOptionWeb {
    pub fee_rate_sat_vb: f64,
    pub fee_sat: u64,
    pub amount_to_sweep_sat: u64,
}

impl From<SweepOption> for BillSweepBTCOptionWeb {
    fn from(val: SweepOption) -> Self {
        Self {
            fee_rate_sat_vb: val.fee_rate_sat_vb,
            fee_sat: val.fee_sat,
            amount_to_sweep_sat: val.amount_to_sweep_sat,
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct BillSweepBTCFundsPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[tsify(type = "string")]
    pub source_address: BitcoinAddress,
    #[tsify(type = "string")]
    pub destination_address: BitcoinAddress,
    pub fee: u64,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillSweepBTCFundsResultWeb {
    pub tx_id: String,
    pub link_to_tx: String,
    pub fee_sat: u64,
    pub sweep_amount: u64,
}

impl From<SweepResult> for BillSweepBTCFundsResultWeb {
    fn from(val: SweepResult) -> Self {
        Self {
            tx_id: val.tx_id,
            link_to_tx: val.link_to_tx,
            fee_sat: val.fee_sat,
            sweep_amount: val.sweep_amount,
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct ResyncBillPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    pub from_nostr: Option<bool>,
}

impl From<BillCombinedBitcoinKey> for BillCombinedBitcoinKeyWeb {
    fn from(val: BillCombinedBitcoinKey) -> Self {
        BillCombinedBitcoinKeyWeb {
            block_id: val.block_id,
            signing_timestamp: val.signing_timestamp,
            payment_op: val.payment_op.into(),
            private_descriptor: val.private_descriptor,
        }
    }
}

#[derive(Tsify, Debug, Clone, Copy, Deserialize)]
pub enum BillsFilterRoleWeb {
    All,
    Payer,
    Payee,
    Contingent,
}

impl From<BillsFilterRoleWeb> for BillsFilterRole {
    fn from(value: BillsFilterRoleWeb) -> Self {
        match value {
            BillsFilterRoleWeb::All => BillsFilterRole::All,
            BillsFilterRoleWeb::Payer => BillsFilterRole::Payer,
            BillsFilterRoleWeb::Payee => BillsFilterRole::Payee,
            BillsFilterRoleWeb::Contingent => BillsFilterRole::Contingent,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct PastEndorseeWeb {
    pub pay_to_the_order_of: LightBillIdentParticipantWeb,
    pub signed: LightSignedByWeb,
    #[tsify(type = "number")]
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddressWeb>,
}

impl From<PastEndorsee> for PastEndorseeWeb {
    fn from(val: PastEndorsee) -> Self {
        PastEndorseeWeb {
            pay_to_the_order_of: val.pay_to_the_order_of.into(),
            signed: val.signed.into(),
            signing_timestamp: val.signing_timestamp,
            signing_address: val.signing_address.map(|s| s.into()),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct LightSignedByWeb {
    pub data: LightBillParticipantWeb,
    pub signatory: Option<LightBillSignatoryWeb>,
}

impl From<LightSignedBy> for LightSignedByWeb {
    fn from(val: LightSignedBy) -> Self {
        LightSignedByWeb {
            data: val.data.into(),
            signatory: val.signatory.map(|s| s.into()),
        }
    }
}

impl From<SignedBy> for LightSignedByWeb {
    fn from(val: SignedBy) -> Self {
        LightSignedByWeb {
            data: LightBillParticipant::from(val.data).into(),
            signatory: val.signatory.map(|s| LightBillSignatoryWeb {
                name: s.name,
                node_id: s.node_id,
            }),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct EndorsementWeb {
    pub pay_to_the_order_of: LightBillParticipantWeb,
    pub signed: LightSignedByWeb,
    #[tsify(type = "number")]
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddressWeb>,
}

impl From<Endorsement> for EndorsementWeb {
    fn from(val: Endorsement) -> Self {
        EndorsementWeb {
            pay_to_the_order_of: val.pay_to_the_order_of.into(),
            signed: val.signed.into(),
            signing_timestamp: val.signing_timestamp,
            signing_address: val.signing_address.map(|s| s.into()),
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct BillsSearchFilterPayload {
    pub filter: BillsSearchFilter,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct DateRange {
    pub from: String,
    pub to: String,
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct BillsSearchFilter {
    pub search_term: Option<String>,
    pub date_range: Option<DateRange>,
    pub role: BillsFilterRoleWeb,
    #[tsify(type = "string[]")]
    #[serde(default)]
    pub participants: Vec<NodeId>,
    #[allow(unused)]
    pub currency: String,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillsResponse {
    pub bills: Vec<BitcreditBillWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillHistoryResponse {
    pub blocks: Vec<BillHistoryBlockWeb>,
}

impl From<BillHistory> for BillHistoryResponse {
    fn from(value: BillHistory) -> Self {
        Self {
            blocks: value.blocks.into_iter().map(|b| b.into()).collect(),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillHistoryBlockWeb {
    #[tsify(type = "number")]
    pub block_id: BlockId,
    pub block_type: BillOpCodeWeb,
    pub pay_to_the_order_of: Option<LightBillParticipantWeb>,
    pub payment_data: Option<BillHistoryBlockPaymentDataWeb>,
    #[tsify(type = "number | undefined")]
    pub request_deadline: Option<Timestamp>,
    pub signed: LightSignedByWeb,
    #[tsify(type = "number")]
    pub signing_timestamp: Timestamp,
    pub signing_address: Option<PostalAddressWeb>,
}

impl From<BillHistoryBlock> for BillHistoryBlockWeb {
    fn from(value: BillHistoryBlock) -> Self {
        Self {
            block_id: value.block_id,
            block_type: value.block_type.into(),
            pay_to_the_order_of: value
                .pay_to_the_order_of
                .map(|pttoo| LightBillParticipant::from(pttoo).into()),
            payment_data: value.payment_data.map(|pd| pd.into()),
            request_deadline: value.request_deadline,
            signed: value.signed.into(),
            signing_timestamp: value.signing_timestamp,
            signing_address: value.signing_address.map(|sa| sa.into()),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillHistoryBlockPaymentDataWeb {
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub payment_address: BitcoinAddress,
}

impl From<BillHistoryBlockPaymentData> for BillHistoryBlockPaymentDataWeb {
    fn from(value: BillHistoryBlockPaymentData) -> Self {
        Self {
            currency: value.sum.currency().code().to_owned(),
            sum: value.sum.as_sat_string(),
            payment_address: value.payment_address,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct LightBillsResponse {
    pub bills: Vec<LightBitcreditBillWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct EndorsementsResponse {
    pub endorsements: Vec<EndorsementWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct PastEndorseesResponse {
    pub past_endorsees: Vec<PastEndorseeWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct PastPaymentsResponse {
    pub past_payments: Vec<PastPaymentResultWeb>,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum PastPaymentResultWeb {
    Sell(PastPaymentDataSellWeb),
    Payment(PastPaymentDataPaymentWeb),
    Recourse(PastPaymentDataRecourseWeb),
}

impl From<PastPaymentResult> for PastPaymentResultWeb {
    fn from(val: PastPaymentResult) -> Self {
        match val {
            PastPaymentResult::Sell(state) => PastPaymentResultWeb::Sell(state.into()),
            PastPaymentResult::Payment(state) => PastPaymentResultWeb::Payment(state.into()),
            PastPaymentResult::Recourse(state) => PastPaymentResultWeb::Recourse(state.into()),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum PaymentStatusWeb {
    Requested(u64),
    Paid(u64),
    Rejected(u64),
    Expired(u64),
}

impl From<PaymentStatus> for PaymentStatusWeb {
    fn from(val: PaymentStatus) -> Self {
        match val {
            PaymentStatus::Requested(ts) => PaymentStatusWeb::Requested(ts.inner()),
            PaymentStatus::Paid(ts) => PaymentStatusWeb::Paid(ts.inner()),
            PaymentStatus::Rejected(ts) => PaymentStatusWeb::Rejected(ts.inner()),
            PaymentStatus::Expired(ts) => PaymentStatusWeb::Expired(ts.inner()),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct PastPaymentDataSellWeb {
    #[tsify(type = "number")]
    pub time_of_request: Timestamp,
    pub buyer: BillParticipantWeb,
    pub seller: BillParticipantWeb,
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub address_to_pay: BitcoinAddress,
    #[tsify(type = "string | undefined")]
    pub private_descriptor_to_spend: Option<BtcDescriptor>,
    pub status: PaymentStatusWeb,
}

impl From<PastPaymentDataSell> for PastPaymentDataSellWeb {
    fn from(val: PastPaymentDataSell) -> Self {
        PastPaymentDataSellWeb {
            time_of_request: val.time_of_request,
            buyer: val.buyer.into(),
            seller: val.seller.into(),
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            address_to_pay: val.address_to_pay,
            private_descriptor_to_spend: val.private_descriptor_to_spend,
            status: val.status.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct PastPaymentDataPaymentWeb {
    #[tsify(type = "number")]
    pub time_of_request: Timestamp,
    pub payer: BillIdentParticipantWeb,
    pub payee: BillParticipantWeb,
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub address_to_pay: BitcoinAddress,
    #[tsify(type = "string | undefined")]
    pub private_descriptor_to_spend: Option<BtcDescriptor>,
    pub status: PaymentStatusWeb,
}
impl From<PastPaymentDataPayment> for PastPaymentDataPaymentWeb {
    fn from(val: PastPaymentDataPayment) -> Self {
        PastPaymentDataPaymentWeb {
            time_of_request: val.time_of_request,
            payer: val.payer.into(),
            payee: val.payee.into(),
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            address_to_pay: val.address_to_pay,
            private_descriptor_to_spend: val.private_descriptor_to_spend,
            status: val.status.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct PastPaymentDataRecourseWeb {
    #[tsify(type = "number")]
    pub time_of_request: Timestamp,
    pub recourser: BillParticipantWeb,
    pub recoursee: BillIdentParticipantWeb,
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub address_to_pay: BitcoinAddress,
    #[tsify(type = "string | undefined")]
    pub private_descriptor_to_spend: Option<BtcDescriptor>,
    pub status: PaymentStatusWeb,
}

impl From<PastPaymentDataRecourse> for PastPaymentDataRecourseWeb {
    fn from(val: PastPaymentDataRecourse) -> Self {
        PastPaymentDataRecourseWeb {
            time_of_request: val.time_of_request,
            recourser: val.recourser.into(),
            recoursee: val.recoursee.into(),
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            address_to_pay: val.address_to_pay,
            private_descriptor_to_spend: val.private_descriptor_to_spend,
            status: val.status.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BitcreditBillWeb {
    #[tsify(type = "string")]
    pub id: BillId,
    pub participants: BillParticipantsWeb,
    pub data: BillDataWeb,
    pub status: BillStatusWeb,
    pub state: BillStateWeb,
    /* Marked for deprecation */
    pub current_waiting_state: Option<BillCurrentWaitingStateWeb>,
    pub actions: BillCallerActionsWeb,
}

impl From<BitcreditBillResult> for BitcreditBillWeb {
    fn from(val: BitcreditBillResult) -> Self {
        BitcreditBillWeb {
            id: val.id,
            participants: val.participants.into(),
            data: val.data.into(),
            status: val.status.into(),
            state: val.state.into(),
            current_waiting_state: val.current_waiting_state.map(|cws| cws.into()),
            actions: val.actions.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillStateWeb {
    pub mint: BillMintStateWeb,
    pub accept: BillAcceptStateWeb,
    pub payment: BillPaymentStateWeb,
}

impl From<BillState> for BillStateWeb {
    fn from(value: BillState) -> Self {
        Self {
            mint: value.mint.into(),
            accept: value.accept.into(),
            payment: value.payment.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub enum BillAcceptStateWeb {
    None,
    Requested(u64),
    Accepted(u64),
    Expired(u64),
    Rejected(u64),
}

impl From<BillAcceptState> for BillAcceptStateWeb {
    fn from(value: BillAcceptState) -> Self {
        match value {
            BillAcceptState::None => BillAcceptStateWeb::None,
            BillAcceptState::Requested(timestamp) => {
                BillAcceptStateWeb::Requested(timestamp.inner())
            }
            BillAcceptState::Accepted(timestamp) => BillAcceptStateWeb::Accepted(timestamp.inner()),
            BillAcceptState::Expired(timestamp) => BillAcceptStateWeb::Expired(timestamp.inner()),
            BillAcceptState::Rejected(timestamp) => BillAcceptStateWeb::Rejected(timestamp.inner()),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub enum BillPaymentStateWeb {
    None,
    Requested(u64),
    Paid(u64),
    Expired(u64),
    Rejected(u64),
}

impl From<BillPaymentState> for BillPaymentStateWeb {
    fn from(value: BillPaymentState) -> Self {
        match value {
            BillPaymentState::None => BillPaymentStateWeb::None,
            BillPaymentState::Requested(timestamp) => {
                BillPaymentStateWeb::Requested(timestamp.inner())
            }
            BillPaymentState::Paid(timestamp) => BillPaymentStateWeb::Paid(timestamp.inner()),
            BillPaymentState::Expired(timestamp) => BillPaymentStateWeb::Expired(timestamp.inner()),
            BillPaymentState::Rejected(timestamp) => {
                BillPaymentStateWeb::Rejected(timestamp.inner())
            }
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub enum BillMintStateWeb {
    None,
    Requested,
}
impl From<BillMintState> for BillMintStateWeb {
    fn from(value: BillMintState) -> Self {
        match value {
            BillMintState::None => BillMintStateWeb::None,
            BillMintState::Requested => BillMintStateWeb::Requested,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub enum BillCurrentWaitingStateWeb {
    Sell(BillWaitingForSellStateWeb),
    Payment(BillWaitingForPaymentStateWeb),
    Recourse(BillWaitingForRecourseStateWeb),
}

impl From<BillCurrentWaitingState> for BillCurrentWaitingStateWeb {
    fn from(val: BillCurrentWaitingState) -> Self {
        match val {
            BillCurrentWaitingState::Sell(state) => BillCurrentWaitingStateWeb::Sell(state.into()),
            BillCurrentWaitingState::Payment(state) => {
                BillCurrentWaitingStateWeb::Payment(state.into())
            }
            BillCurrentWaitingState::Recourse(state) => {
                BillCurrentWaitingStateWeb::Recourse(state.into())
            }
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillWaitingStatePaymentDataWeb {
    #[tsify(type = "number")]
    pub time_of_request: Timestamp,
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub address_to_pay: BitcoinAddress,
    pub tx_id: Option<String>,
    pub in_mempool: bool,
    pub confirmations: u64,
    #[tsify(type = "number | undefined")]
    pub payment_deadline: Option<Timestamp>,
}

impl From<BillWaitingStatePaymentData> for BillWaitingStatePaymentDataWeb {
    fn from(val: BillWaitingStatePaymentData) -> Self {
        BillWaitingStatePaymentDataWeb {
            time_of_request: val.time_of_request,
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            address_to_pay: val.address_to_pay,
            tx_id: val.tx_id,
            in_mempool: val.in_mempool,
            confirmations: val.confirmations,
            payment_deadline: val.payment_deadline,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillWaitingForSellStateWeb {
    pub buyer: BillParticipantWeb,
    pub seller: BillParticipantWeb,
    pub payment_data: BillWaitingStatePaymentDataWeb,
}

impl From<BillWaitingForSellState> for BillWaitingForSellStateWeb {
    fn from(val: BillWaitingForSellState) -> Self {
        BillWaitingForSellStateWeb {
            buyer: val.buyer.into(),
            seller: val.seller.into(),
            payment_data: val.payment_data.into(),
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillWaitingForPaymentStateWeb {
    pub payer: BillIdentParticipantWeb,
    pub payee: BillParticipantWeb,
    pub payment_data: BillWaitingStatePaymentDataWeb,
}

impl From<BillWaitingForPaymentState> for BillWaitingForPaymentStateWeb {
    fn from(val: BillWaitingForPaymentState) -> Self {
        BillWaitingForPaymentStateWeb {
            payer: val.payer.into(),
            payee: val.payee.into(),
            payment_data: val.payment_data.into(),
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillWaitingForRecourseStateWeb {
    pub recourser: BillParticipantWeb,
    pub recoursee: BillIdentParticipantWeb,
    pub payment_data: BillWaitingStatePaymentDataWeb,
}
impl From<BillWaitingForRecourseState> for BillWaitingForRecourseStateWeb {
    fn from(val: BillWaitingForRecourseState) -> Self {
        BillWaitingForRecourseStateWeb {
            recourser: val.recourser.into(),
            recoursee: val.recoursee.into(),
            payment_data: val.payment_data.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillStatusWeb {
    /* Marked for deprecation */
    pub acceptance: BillAcceptanceStatusWeb,
    /* Marked for deprecation */
    pub payment: BillPaymentStatusWeb,
    /* Marked for deprecation */
    pub sell: BillSellStatusWeb,
    /* Marked for deprecation */
    pub recourse: BillRecourseStatusWeb,
    pub mint: BillMintStatusWeb,
    /* Marked for deprecation */
    pub redeemed_funds_available: bool,
    /* Marked for deprecation */
    pub has_requested_funds: bool,
    /* Marked for deprecation */
    #[tsify(type = "number")]
    /* Marked for deprecation */
    pub last_block_time: Timestamp,
}

impl From<BillStatus> for BillStatusWeb {
    fn from(val: BillStatus) -> Self {
        BillStatusWeb {
            acceptance: val.acceptance.into(),
            payment: val.payment.into(),
            sell: val.sell.into(),
            recourse: val.recourse.into(),
            mint: val.mint.into(),
            redeemed_funds_available: val.redeemed_funds_available,
            has_requested_funds: val.has_requested_funds,
            last_block_time: val.last_block_time,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillAcceptanceStatusWeb {
    #[tsify(type = "number | undefined")]
    pub time_of_request_to_accept: Option<Timestamp>,
    pub requested_to_accept: bool,
    pub accepted: bool,
    pub request_to_accept_timed_out: bool,
    pub rejected_to_accept: bool,
    #[tsify(type = "number | undefined")]
    pub acceptance_deadline_timestamp: Option<Timestamp>,
}

impl From<BillAcceptanceStatus> for BillAcceptanceStatusWeb {
    fn from(val: BillAcceptanceStatus) -> Self {
        BillAcceptanceStatusWeb {
            time_of_request_to_accept: val.time_of_request_to_accept,
            requested_to_accept: val.requested_to_accept,
            accepted: val.accepted,
            request_to_accept_timed_out: val.request_to_accept_timed_out,
            rejected_to_accept: val.rejected_to_accept,
            acceptance_deadline_timestamp: val.acceptance_deadline_timestamp,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillPaymentStatusWeb {
    #[tsify(type = "number | undefined")]
    pub time_of_request_to_pay: Option<Timestamp>,
    pub requested_to_pay: bool,
    pub paid: bool,
    pub request_to_pay_timed_out: bool,
    pub rejected_to_pay: bool,
    #[tsify(type = "number | undefined")]
    pub payment_deadline_timestamp: Option<Timestamp>,
}
impl From<BillPaymentStatus> for BillPaymentStatusWeb {
    fn from(val: BillPaymentStatus) -> Self {
        BillPaymentStatusWeb {
            time_of_request_to_pay: val.time_of_request_to_pay,
            requested_to_pay: val.requested_to_pay,
            paid: val.paid,
            request_to_pay_timed_out: val.request_to_pay_timed_out,
            rejected_to_pay: val.rejected_to_pay,
            payment_deadline_timestamp: val.payment_deadline_timestamp,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillSellStatusWeb {
    #[tsify(type = "number | undefined")]
    pub time_of_last_offer_to_sell: Option<Timestamp>,
    pub sold: bool,
    pub offered_to_sell: bool,
    pub offer_to_sell_timed_out: bool,
    pub rejected_offer_to_sell: bool,
    #[tsify(type = "number | undefined")]
    pub buying_deadline_timestamp: Option<Timestamp>,
}
impl From<BillSellStatus> for BillSellStatusWeb {
    fn from(val: BillSellStatus) -> Self {
        BillSellStatusWeb {
            time_of_last_offer_to_sell: val.time_of_last_offer_to_sell,
            sold: val.sold,
            offered_to_sell: val.offered_to_sell,
            offer_to_sell_timed_out: val.offer_to_sell_timed_out,
            rejected_offer_to_sell: val.rejected_offer_to_sell,
            buying_deadline_timestamp: val.buying_deadline_timestamp,
        }
    }
}

/* Marked for deprecation */
#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillRecourseStatusWeb {
    #[tsify(type = "number | undefined")]
    pub time_of_last_request_to_recourse: Option<Timestamp>,
    pub recoursed: bool,
    pub requested_to_recourse: bool,
    pub request_to_recourse_timed_out: bool,
    pub rejected_request_to_recourse: bool,
    #[tsify(type = "number | undefined")]
    pub recourse_deadline_timestamp: Option<Timestamp>,
}

impl From<BillRecourseStatus> for BillRecourseStatusWeb {
    fn from(val: BillRecourseStatus) -> Self {
        BillRecourseStatusWeb {
            time_of_last_request_to_recourse: val.time_of_last_request_to_recourse,
            recoursed: val.recoursed,
            requested_to_recourse: val.requested_to_recourse,
            request_to_recourse_timed_out: val.request_to_recourse_timed_out,
            rejected_request_to_recourse: val.rejected_request_to_recourse,
            recourse_deadline_timestamp: val.recourse_deadline_timestamp,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillMintStatusWeb {
    pub has_mint_requests: bool,
}

impl From<BillMintStatus> for BillMintStatusWeb {
    fn from(val: BillMintStatus) -> Self {
        BillMintStatusWeb {
            has_mint_requests: val.has_mint_requests,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillDataWeb {
    #[tsify(type = "number")]
    pub time_of_drawing: Timestamp,
    #[tsify(type = "string")]
    pub issue_date: Date,
    #[tsify(type = "number")]
    pub time_of_maturity: Timestamp,
    #[tsify(type = "string")]
    pub maturity_date: Date,
    #[tsify(type = "string")]
    pub country_of_issuing: Country,
    #[tsify(type = "string")]
    pub city_of_issuing: City,
    #[tsify(type = "string")]
    pub country_of_payment: Country,
    #[tsify(type = "string")]
    pub city_of_payment: City,
    pub currency: String,
    pub sum: String,
    pub files: Vec<FileWeb>,
    pub active_notification: Option<NotificationWeb>,
}

impl From<BillData> for BillDataWeb {
    fn from(val: BillData) -> Self {
        BillDataWeb {
            time_of_drawing: val.time_of_drawing,
            issue_date: val.issue_date,
            time_of_maturity: val.time_of_maturity,
            maturity_date: val.maturity_date,
            country_of_issuing: val.country_of_issuing,
            city_of_issuing: val.city_of_issuing,
            country_of_payment: val.country_of_payment,
            city_of_payment: val.city_of_payment,
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            files: val.files.into_iter().map(|f| f.into()).collect(),
            active_notification: val.active_notification.map(|an| an.into()),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillParticipantsWeb {
    pub drawee: BillIdentParticipantWeb,
    pub drawer: BillIdentParticipantWeb,
    pub payee: BillParticipantWeb,
    pub endorsee: Option<BillParticipantWeb>,
    pub endorsements_count: u64,
    #[tsify(type = "string[]")]
    pub all_participant_node_ids: Vec<NodeId>,
}

impl From<BillParticipants> for BillParticipantsWeb {
    fn from(val: BillParticipants) -> Self {
        BillParticipantsWeb {
            drawee: val.drawee.into(),
            drawer: val.drawer.into(),
            payee: val.payee.into(),
            endorsee: val.endorsee.map(|e| e.into()),
            endorsements_count: val.endorsements_count,
            all_participant_node_ids: val.all_participant_node_ids,
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillCallerActionsWeb {
    pub bill_actions: Vec<BillCallerBillActionWeb>,
    pub payment_actions: Vec<BillCallerPaymentActionWeb>,
}

impl From<BillCallerActions> for BillCallerActionsWeb {
    fn from(value: BillCallerActions) -> Self {
        Self {
            bill_actions: value.bill_actions.into_iter().map(|ba| ba.into()).collect(),
            payment_actions: value
                .payment_actions
                .into_iter()
                .map(|ba| ba.into())
                .collect(),
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum BillCallerBillActionWeb {
    RequestAcceptance,
    Accept,
    RequestToPay,
    OfferToSell,
    Sell,
    Endorse,
    RequestRecourseForAcceptance,
    RequestRecourseForPayment,
    Recourse,
    Mint,
    RejectAcceptance,
    RejectPayment,
    RejectBuying,
    RejectPaymentForRecourse,
}

impl From<BillCallerBillAction> for BillCallerBillActionWeb {
    fn from(value: BillCallerBillAction) -> Self {
        match value {
            BillCallerBillAction::RequestAcceptance => BillCallerBillActionWeb::RequestAcceptance,
            BillCallerBillAction::Accept => BillCallerBillActionWeb::Accept,
            BillCallerBillAction::RequestToPay => BillCallerBillActionWeb::RequestToPay,
            BillCallerBillAction::OfferToSell => BillCallerBillActionWeb::OfferToSell,
            BillCallerBillAction::Sell => BillCallerBillActionWeb::Sell,
            BillCallerBillAction::Endorse => BillCallerBillActionWeb::Endorse,
            BillCallerBillAction::RequestRecourseForAcceptance => {
                BillCallerBillActionWeb::RequestRecourseForAcceptance
            }
            BillCallerBillAction::RequestRecourseForPayment => {
                BillCallerBillActionWeb::RequestRecourseForPayment
            }
            BillCallerBillAction::Recourse => BillCallerBillActionWeb::Recourse,
            BillCallerBillAction::Mint => BillCallerBillActionWeb::Mint,
            BillCallerBillAction::RejectAcceptance => BillCallerBillActionWeb::RejectAcceptance,
            BillCallerBillAction::RejectPayment => BillCallerBillActionWeb::RejectPayment,
            BillCallerBillAction::RejectBuying => BillCallerBillActionWeb::RejectBuying,
            BillCallerBillAction::RejectPaymentForRecourse => {
                BillCallerBillActionWeb::RejectPaymentForRecourse
            }
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum BillCallerPaymentActionWeb {
    Pay(BillCallerPaymentWeb),
    CheckPayment(BillCallerPaymentWeb),
}

impl From<BillCallerPaymentAction> for BillCallerPaymentActionWeb {
    fn from(value: BillCallerPaymentAction) -> Self {
        match value {
            BillCallerPaymentAction::Pay(bill_caller_payment) => {
                BillCallerPaymentActionWeb::Pay(bill_caller_payment.into())
            }
            BillCallerPaymentAction::CheckPayment(bill_caller_payment) => {
                BillCallerPaymentActionWeb::CheckPayment(bill_caller_payment.into())
            }
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub enum BillCallerPaymentWeb {
    Sell {
        buyer: BillParticipantWeb,
        seller: BillParticipantWeb,
        state: BillCallerPaymentStateWeb,
    },
    Payment {
        payer: BillIdentParticipantWeb,
        payee: BillParticipantWeb,
        state: BillCallerPaymentStateWeb,
    },
    Recourse {
        recourser: BillParticipantWeb,
        recoursee: BillIdentParticipantWeb,
        state: BillCallerPaymentStateWeb,
    },
}

impl From<BillCallerPayment> for BillCallerPaymentWeb {
    fn from(value: BillCallerPayment) -> Self {
        match value {
            BillCallerPayment::Sell {
                buyer,
                seller,
                state,
            } => BillCallerPaymentWeb::Sell {
                buyer: buyer.into(),
                seller: seller.into(),
                state: state.into(),
            },
            BillCallerPayment::Payment {
                payer,
                payee,
                state,
            } => BillCallerPaymentWeb::Payment {
                payer: payer.into(),
                payee: payee.into(),
                state: state.into(),
            },
            BillCallerPayment::Recourse {
                recourser,
                recoursee,
                state,
            } => BillCallerPaymentWeb::Recourse {
                recourser: recourser.into(),
                recoursee: recoursee.into(),
                state: state.into(),
            },
        }
    }
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct BillCallerPaymentStateWeb {
    #[tsify(type = "number")]
    pub time_of_request: Timestamp,
    pub currency: String,
    pub sum: String,
    #[tsify(type = "string")]
    pub address_to_pay: BitcoinAddress,
    pub status: PaymentStatusWeb,
    #[tsify(type = "number")]
    pub payment_deadline: Timestamp,
    pub tx_id: Option<String>,
    pub in_mempool: bool,
    pub confirmations: u64,
    // only set if we're receiver
    #[tsify(type = "string | undefined")]
    pub private_descriptor_to_spend: Option<BtcDescriptor>,
}

impl From<BillCallerPaymentState> for BillCallerPaymentStateWeb {
    fn from(value: BillCallerPaymentState) -> Self {
        Self {
            time_of_request: value.time_of_request,
            currency: value.sum.currency().code().to_owned(),
            sum: value.sum.as_sat_string(),
            address_to_pay: value.address_to_pay,
            status: value.status.into(),
            payment_deadline: value.payment_deadline,
            tx_id: value.tx_id,
            in_mempool: value.in_mempool,
            confirmations: value.confirmations,
            private_descriptor_to_spend: value.private_descriptor_to_spend,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LightBitcreditBillWeb {
    #[tsify(type = "string")]
    pub id: BillId,
    pub drawee: LightBillIdentParticipantWeb,
    pub drawer: LightBillIdentParticipantWeb,
    pub payee: LightBillParticipantWeb,
    pub endorsee: Option<LightBillParticipantWeb>,
    pub active_notification: Option<NotificationWeb>,
    pub sum: String,
    pub currency: String,
    #[tsify(type = "string")]
    pub issue_date: Date,
    #[tsify(type = "number")]
    pub time_of_drawing: Timestamp,
    #[tsify(type = "number")]
    pub time_of_maturity: Timestamp,
    #[tsify(type = "number")]
    pub last_block_time: Timestamp,
}

impl From<LightBitcreditBillResult> for LightBitcreditBillWeb {
    fn from(val: LightBitcreditBillResult) -> Self {
        LightBitcreditBillWeb {
            id: val.id,
            drawee: val.drawee.into(),
            drawer: val.drawer.into(),
            payee: val.payee.into(),
            endorsee: val.endorsee.map(|e| e.into()),
            active_notification: val.active_notification.map(|n| n.into()),
            currency: val.sum.currency().code().to_owned(),
            sum: val.sum.as_sat_string(),
            issue_date: val.issue_date,
            time_of_drawing: val.time_of_drawing,
            time_of_maturity: val.time_of_maturity,
            last_block_time: val.last_block_time,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub enum BillParticipantWeb {
    Anon(BillAnonParticipantWeb),
    Ident(BillIdentParticipantWeb),
}

impl From<BillParticipant> for BillParticipantWeb {
    fn from(val: BillParticipant) -> Self {
        match val {
            BillParticipant::Ident(data) => BillParticipantWeb::Ident(data.into()),
            BillParticipant::Anon(data) => BillParticipantWeb::Anon(data.into()),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillAnonParticipantWeb {
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string[]")]
    pub nostr_relays: Vec<url::Url>,
}

impl From<BillAnonParticipant> for BillAnonParticipantWeb {
    fn from(val: BillAnonParticipant) -> Self {
        BillAnonParticipantWeb {
            node_id: val.node_id,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct BillIdentParticipantWeb {
    pub t: ContactTypeWeb,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    #[tsify(type = "string")]
    pub name: Name,
    pub postal_address: PostalAddressWeb,
    #[tsify(type = "string | undefined")]
    pub email: Option<Email>,
    #[tsify(type = "string[]")]
    pub nostr_relays: Vec<url::Url>,
}

impl From<BillIdentParticipant> for BillIdentParticipantWeb {
    fn from(val: BillIdentParticipant) -> Self {
        BillIdentParticipantWeb {
            t: val.t.into(),
            name: val.name,
            node_id: val.node_id,
            postal_address: val.postal_address.into(),
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LightBillIdentParticipantWithAddressWeb {
    pub t: ContactTypeWeb,
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string")]
    pub node_id: NodeId,
    pub postal_address: PostalAddressWeb,
}

impl From<LightBillIdentParticipantWithAddress> for LightBillIdentParticipantWithAddressWeb {
    fn from(val: LightBillIdentParticipantWithAddress) -> Self {
        LightBillIdentParticipantWithAddressWeb {
            t: val.t.into(),
            name: val.name,
            node_id: val.node_id,
            postal_address: val.postal_address.into(),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub enum LightBillParticipantWeb {
    Anon(LightBillAnonParticipantWeb),
    Ident(LightBillIdentParticipantWithAddressWeb),
}

impl From<LightBillParticipant> for LightBillParticipantWeb {
    fn from(val: LightBillParticipant) -> Self {
        match val {
            LightBillParticipant::Ident(data) => LightBillParticipantWeb::Ident(data.into()),
            LightBillParticipant::Anon(data) => LightBillParticipantWeb::Anon(data.into()),
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LightBillAnonParticipantWeb {
    #[tsify(type = "string")]
    pub node_id: NodeId,
}

impl From<LightBillAnonParticipant> for LightBillAnonParticipantWeb {
    fn from(val: LightBillAnonParticipant) -> Self {
        LightBillAnonParticipantWeb {
            node_id: val.node_id,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LightBillIdentParticipantWeb {
    pub t: ContactTypeWeb,
    #[tsify(type = "string")]
    pub name: Name,
    #[tsify(type = "string")]
    pub node_id: NodeId,
}

impl From<LightBillIdentParticipant> for LightBillIdentParticipantWeb {
    fn from(val: LightBillIdentParticipant) -> Self {
        LightBillIdentParticipantWeb {
            t: val.t.into(),
            name: val.name,
            node_id: val.node_id,
        }
    }
}

impl From<BillIdentParticipant> for LightBillIdentParticipantWeb {
    fn from(val: BillIdentParticipant) -> Self {
        LightBillIdentParticipantWeb {
            t: val.t.into(),
            name: val.name,
            node_id: val.node_id,
        }
    }
}

#[derive(Tsify, Debug, Serialize, Clone)]
pub struct LightBillSignatoryWeb {
    #[tsify(type = "string | undefined")]
    pub name: Option<Name>,
    #[tsify(type = "string")]
    pub node_id: NodeId,
}

impl From<LightBillSignatory> for LightBillSignatoryWeb {
    fn from(val: LightBillSignatory) -> Self {
        Self {
            name: val.name,
            node_id: val.node_id,
        }
    }
}

#[derive(Tsify, Debug, Clone, Deserialize)]
pub struct ShareBillWithCourtPayload {
    #[tsify(type = "string")]
    pub bill_id: BillId,
    #[tsify(type = "string")]
    pub court_node_id: NodeId,
}

#[derive(Tsify, Debug, Copy, Clone, Serialize)]
pub enum BillOpCodeWeb {
    Issue,
    Accept,
    Endorse,
    RequestToAccept,
    RequestToPay,
    OfferToSell,
    Sell,
    Mint,
    RejectToAccept,
    RejectToPay,
    RejectToBuy,
    RejectToPayRecourse,
    RequestRecourse,
    Recourse,
}

impl From<BillOpCode> for BillOpCodeWeb {
    fn from(value: BillOpCode) -> Self {
        match value {
            BillOpCode::Issue => BillOpCodeWeb::Issue,
            BillOpCode::Accept => BillOpCodeWeb::Accept,
            BillOpCode::Endorse => BillOpCodeWeb::Endorse,
            BillOpCode::RequestToAccept => BillOpCodeWeb::RequestToAccept,
            BillOpCode::RequestToPay => BillOpCodeWeb::RequestToPay,
            BillOpCode::OfferToSell => BillOpCodeWeb::OfferToSell,
            BillOpCode::Sell => BillOpCodeWeb::Sell,
            BillOpCode::Mint => BillOpCodeWeb::Mint,
            BillOpCode::RejectToAccept => BillOpCodeWeb::RejectToAccept,
            BillOpCode::RejectToPay => BillOpCodeWeb::RejectToPay,
            BillOpCode::RejectToBuy => BillOpCodeWeb::RejectToBuy,
            BillOpCode::RejectToPayRecourse => BillOpCodeWeb::RejectToPayRecourse,
            BillOpCode::RequestRecourse => BillOpCodeWeb::RequestRecourse,
            BillOpCode::Recourse => BillOpCodeWeb::Recourse,
        }
    }
}
