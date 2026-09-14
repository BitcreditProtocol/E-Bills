use bcr_ebill_api::service::bill_service::Error as BillServiceError;
use bcr_ebill_api::service::{
    Error as ServiceError, transport_service::Error as NotificationServiceError,
};
use bcr_ebill_core::application::ValidationError;
use bcr_ebill_core::protocol::{ProtocolValidationError, crypto};
use log::error;
use serde::Serialize;
use thiserror::Error;
use tsify::Tsify;
use wasm_bindgen::prelude::*;

#[derive(Debug, Error)]
pub enum WasmError {
    #[error("service error: {0}")]
    Service(#[from] ServiceError),

    #[error("bill service error: {0}")]
    BillService(#[from] BillServiceError),

    #[error("notification service error: {0}")]
    NotificationService(#[from] NotificationServiceError),

    #[error("wasm serialization error: {0}")]
    WasmSerialization(#[from] serde_wasm_bindgen::Error),

    #[error("crypto error: {0}")]
    Crypto(#[from] crypto::Error),

    #[error("persistence error: {0}")]
    Persistence(#[from] bcr_ebill_persistence::Error),

    #[error("api init error: {0}")]
    Init(#[from] anyhow::Error),

    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("Protocol Validation error: {0}")]
    ProtocolValidation(#[from] ProtocolValidationError),
}

#[derive(Tsify, Debug, Clone, Serialize)]
enum JsErrorType {
    FieldEmpty,
    FieldInvalid,
    InvalidSum,
    InvalidCurrency,
    InvalidBitcoinAddress,
    InvalidBitcoinDescriptor,
    InvalidContentType,
    IdentityCantBeAnon,
    InvalidContactType,
    InvalidIdentityType,
    InvalidDate,
    InvalidCountry,
    InvalidTimestamp,
    DeadlineBeforeMinimum,
    SelfDraftedBillCantBeBlank,
    RequestToMintForBillAndMintAlreadyActive,
    SignerCantBeAnon,
    ContactIsAnonymous,
    InvalidContact,
    InvalidMint,
    IssueDateAfterMaturityDate,
    MaturityDateInThePast,
    RequestToPayBeforeMaturityDate,
    InvalidFileUploadId,
    InvalidNodeId,
    InvalidBillId,
    InvalidBillType,
    DraweeCantBePayee,
    EndorserCantBeEndorsee,
    BuyerCantBeSeller,
    RecourserCantBeRecoursee,
    DraweeNotInContacts,
    PayeeNotInContacts,
    MintNotInContacts,
    BuyerNotInContacts,
    EndorseeNotInContacts,
    RecourseeNotInContacts,
    NoPaymentForSweep,
    CancelMintRequestNotPending,
    RejectMintRequestNotOffered,
    AcceptMintRequestNotOffered,
    AcceptMintOfferExpired,
    NoConfirmedEmailForIdentIdentity,
    NoFileForFileUploadId,
    NotFound,
    ExternalApi,
    Crypto,
    Persistence,
    Blockchain,
    Protocol,
    InvalidRelayUrl,
    Serialization,
    Init,
    // notification
    NotificationNetwork,
    NotificationMessage,
    //bill
    InvalidOperation,
    BillAlreadyAccepted,
    BillWasRejectedToAccept,
    BillAcceptanceExpired,
    BillWasRejectedToPay,
    BillPaymentExpired,
    BillWasRecoursedToTheEnd,
    BillAlreadyRequestedToAccept,
    BillNotAccepted,
    CallerIsNotDrawee,
    CallerIsNotHolder,
    CallerIsNotRecoursee,
    CallerIsNotBuyer,
    RequestAlreadyRejected,
    BillAlreadyPaid,
    BillWasNotRequestedToPay,
    BillWasNotOfferedToSell,
    BillRequestToAcceptDidNotExpireAndWasNotRejected,
    BillRequestToPayDidNotExpireAndWasNotRejected,
    RecourseeNotPastHolder,
    BillWasNotRequestedToRecourse,
    BillIsNotRequestedToRecourseAndWaitingForPayment,
    BillIsNotOfferToSellWaitingForPayment,
    BillSellDataInvalid,
    BillRecourseDataInvalid,
    BillIsRequestedToPayAndWaitingForPayment,
    BillIsOfferedToSellAndWaitingForPayment,
    BillIsInRecourseAndWaitingForPayment,
    // general
    SignatoryNotInContacts,
    SignatoryAlreadySignatory,
    CantRemoveLastSignatory,
    NotASignatory,
    NotARemovedOrRejectedSignatory,
    NotInvitedAsSignatory,
    NoSignerIdentityProof,
    FileIsTooBig,
    FileIsEmpty,
    TooManyFiles,
    InvalidFileName,
    UnknownNodeId,
    CallerMustBeSignatory,
    CallerMustBeCreator,
    CallerMustBeIdentifiedCreator,
    InvalidIdentityProof,
    InvalidReferenceBlock,
    MintRequestToPayWithoutCustomAddress,
    InvalidSignature,
    InvalidHash,
    InvalidUrl,
    InvalidIdentityProofStatus,
    Json,
    InvalidBillAction,
    InvalidCompanyAction,
    InvalidIdentityAction,
    CompanySignerCreatorMismatch,
    InvalidMintRequestId,
}

#[derive(Tsify, Debug, Clone, Serialize)]
pub struct JsErrorData {
    error: JsErrorType,
    message: String,
    code: u16,
}

impl From<WasmError> for JsValue {
    fn from(error: WasmError) -> JsValue {
        serde_wasm_bindgen::to_value(&JsErrorData::from(error)).expect("can serialize error")
    }
}

impl From<WasmError> for JsErrorData {
    fn from(error: WasmError) -> JsErrorData {
        error!("{error}");
        match error {
            WasmError::Service(e) => match e {
                ServiceError::NotFound => err_404(e, JsErrorType::NotFound),
                ServiceError::TransportService(e) => notification_service_error_data(e),
                ServiceError::BillService(e) => bill_service_error_data(e),
                ServiceError::Validation(e) => validation_error_data(e),
                ServiceError::ExternalApi(e) => err_500(e, JsErrorType::ExternalApi),
                ServiceError::CryptoUtil(e) => err_500(e, JsErrorType::Crypto),
                ServiceError::Persistence(e) => err_500(e, JsErrorType::Persistence),
                ServiceError::Protocol(e) => err_500(e, JsErrorType::Protocol),
                ServiceError::Json(e) => err_500(e, JsErrorType::Json),
            },
            WasmError::BillService(e) => bill_service_error_data(e),
            WasmError::Validation(e) => validation_error_data(e),
            WasmError::ProtocolValidation(e) => protocol_validation_error_data(e),
            WasmError::NotificationService(e) => notification_service_error_data(e),
            WasmError::WasmSerialization(e) => err_500(e, JsErrorType::Serialization),
            WasmError::Crypto(e) => err_500(e, JsErrorType::Crypto),
            WasmError::Persistence(e) => err_500(e, JsErrorType::Persistence),
            WasmError::Init(e) => err_500(e, JsErrorType::Init),
        }
    }
}

fn notification_service_error_data(e: NotificationServiceError) -> JsErrorData {
    match e {
        NotificationServiceError::Network(e) => err_500(e, JsErrorType::NotificationNetwork),
        NotificationServiceError::Message(e) => err_500(e, JsErrorType::NotificationMessage),
        NotificationServiceError::Persistence(e) => err_500(e, JsErrorType::Persistence),
        NotificationServiceError::Crypto(e) => err_500(e, JsErrorType::Crypto),
        NotificationServiceError::Blockchain(e) => err_500(e, JsErrorType::Blockchain),
        NotificationServiceError::Validation(e) => validation_error_data(e),
        NotificationServiceError::ExternalApi(e) => err_500(e, JsErrorType::ExternalApi),
        NotificationServiceError::NotFound => err_404(e, JsErrorType::NotFound),
    }
}

fn bill_service_error_data(e: BillServiceError) -> JsErrorData {
    match e {
        BillServiceError::Validation(e) => validation_error_data(e),
        BillServiceError::NotFound => err_404(e, JsErrorType::NotFound),
        BillServiceError::Persistence(e) => err_500(e, JsErrorType::Persistence),
        BillServiceError::ExternalApi(e) => err_500(e, JsErrorType::ExternalApi),
        BillServiceError::Protocol(e) => err_500(e, JsErrorType::Protocol),
        BillServiceError::Cryptography(e) => err_500(e, JsErrorType::Crypto),
        BillServiceError::Notification(e) => notification_service_error_data(e),
        BillServiceError::Json(e) => err_500(e, JsErrorType::Serialization),
    }
}

fn validation_error_data(e: ValidationError) -> JsErrorData {
    match e {
        ValidationError::Protocol(e) => protocol_validation_error_data(e),
        ValidationError::RequestToMintForBillAndMintAlreadyActive => {
            err_400(e, JsErrorType::RequestToMintForBillAndMintAlreadyActive)
        }
        ValidationError::ContactIsAnonymous(_) => err_400(e, JsErrorType::ContactIsAnonymous),
        ValidationError::InvalidContact(_) => err_400(e, JsErrorType::InvalidContact),
        ValidationError::InvalidMint(_) => err_400(e, JsErrorType::InvalidMint),
        ValidationError::InvalidBillType => err_400(e, JsErrorType::InvalidBillType),
        ValidationError::SignatoryNotInContacts(_) => {
            err_400(e, JsErrorType::SignatoryNotInContacts)
        }
        ValidationError::UnknownNodeId(_) => err_400(e, JsErrorType::UnknownNodeId),
        ValidationError::Blockchain(e) => err_500(e, JsErrorType::Blockchain),
        ValidationError::InvalidIdentityProofStatus(_) => {
            err_400(e, JsErrorType::InvalidIdentityProofStatus)
        }
        ValidationError::InvalidOperation => err_400(e, JsErrorType::InvalidOperation),
        ValidationError::NoFileForFileUploadId => err_400(e, JsErrorType::NoFileForFileUploadId),
        ValidationError::DraweeNotInContacts => err_400(e, JsErrorType::DraweeNotInContacts),
        ValidationError::PayeeNotInContacts => err_400(e, JsErrorType::PayeeNotInContacts),
        ValidationError::BuyerNotInContacts => err_400(e, JsErrorType::BuyerNotInContacts),
        ValidationError::EndorseeNotInContacts => err_400(e, JsErrorType::EndorseeNotInContacts),
        ValidationError::MintNotInContacts => err_400(e, JsErrorType::MintNotInContacts),
        ValidationError::RecourseeNotInContacts => err_400(e, JsErrorType::RecourseeNotInContacts),
        ValidationError::NoPaymentForSweep => err_400(e, JsErrorType::NoPaymentForSweep),
        ValidationError::CancelMintRequestNotPending => {
            err_400(e, JsErrorType::CancelMintRequestNotPending)
        }
        ValidationError::RejectMintRequestNotOffered => {
            err_400(e, JsErrorType::RejectMintRequestNotOffered)
        }
        ValidationError::AcceptMintRequestNotOffered => {
            err_400(e, JsErrorType::AcceptMintRequestNotOffered)
        }
        ValidationError::AcceptMintOfferExpired => err_400(e, JsErrorType::AcceptMintOfferExpired),
        ValidationError::NoConfirmedEmailForIdentIdentity => {
            err_400(e, JsErrorType::NoConfirmedEmailForIdentIdentity)
        }
    }
}

fn protocol_validation_error_data(e: ProtocolValidationError) -> JsErrorData {
    match e {
        ProtocolValidationError::FieldEmpty(_) => err_400(e, JsErrorType::FieldEmpty),
        ProtocolValidationError::FieldInvalid(_) => err_400(e, JsErrorType::FieldInvalid),
        ProtocolValidationError::InvalidSum => err_400(e, JsErrorType::InvalidSum),
        ProtocolValidationError::InvalidBitcoinAddress => {
            err_400(e, JsErrorType::InvalidBitcoinAddress)
        }
        ProtocolValidationError::InvalidBitcoinDescriptor => {
            err_400(e, JsErrorType::InvalidBitcoinDescriptor)
        }
        ProtocolValidationError::InvalidCurrency => err_400(e, JsErrorType::InvalidCurrency),
        ProtocolValidationError::InvalidContactType => err_400(e, JsErrorType::InvalidContactType),
        ProtocolValidationError::InvalidIdentityType => {
            err_400(e, JsErrorType::InvalidIdentityType)
        }
        ProtocolValidationError::InvalidContentType => err_400(e, JsErrorType::InvalidContentType),
        ProtocolValidationError::InvalidDate => err_400(e, JsErrorType::InvalidDate),
        ProtocolValidationError::InvalidCountry => err_400(e, JsErrorType::InvalidCountry),
        ProtocolValidationError::InvalidTimestamp => err_400(e, JsErrorType::InvalidTimestamp),
        ProtocolValidationError::DeadlineBeforeMinimum => {
            err_400(e, JsErrorType::DeadlineBeforeMinimum)
        }
        ProtocolValidationError::SelfDraftedBillCantBeBlank => {
            err_400(e, JsErrorType::SelfDraftedBillCantBeBlank)
        }
        ProtocolValidationError::IdentityCantBeAnon => err_400(e, JsErrorType::IdentityCantBeAnon),
        ProtocolValidationError::SignerCantBeAnon => err_400(e, JsErrorType::SignerCantBeAnon),
        ProtocolValidationError::MaturityDateInThePast => {
            err_400(e, JsErrorType::MaturityDateInThePast)
        }
        ProtocolValidationError::IssueDateAfterMaturityDate => {
            err_400(e, JsErrorType::IssueDateAfterMaturityDate)
        }
        ProtocolValidationError::RequestToPayBeforeMaturityDate => {
            err_400(e, JsErrorType::RequestToPayBeforeMaturityDate)
        }
        ProtocolValidationError::InvalidFileUploadId => {
            err_400(e, JsErrorType::InvalidFileUploadId)
        }
        ProtocolValidationError::InvalidNodeId => err_400(e, JsErrorType::InvalidNodeId),
        ProtocolValidationError::InvalidBillId => err_400(e, JsErrorType::InvalidBillId),
        ProtocolValidationError::DraweeCantBePayee => err_400(e, JsErrorType::DraweeCantBePayee),
        ProtocolValidationError::EndorserCantBeEndorsee => {
            err_400(e, JsErrorType::EndorserCantBeEndorsee)
        }
        ProtocolValidationError::BuyerCantBeSeller => err_400(e, JsErrorType::BuyerCantBeSeller),
        ProtocolValidationError::RecourserCantBeRecoursee => {
            err_400(e, JsErrorType::RecourserCantBeRecoursee)
        }
        ProtocolValidationError::BillAlreadyAccepted => {
            err_400(e, JsErrorType::BillAlreadyAccepted)
        }
        ProtocolValidationError::BillWasRejectedToAccept => {
            err_400(e, JsErrorType::BillWasRejectedToAccept)
        }
        ProtocolValidationError::BillAcceptanceExpired => {
            err_400(e, JsErrorType::BillAcceptanceExpired)
        }
        ProtocolValidationError::BillWasRejectedToPay => {
            err_400(e, JsErrorType::BillWasRejectedToPay)
        }
        ProtocolValidationError::BillPaymentExpired => err_400(e, JsErrorType::BillPaymentExpired),
        ProtocolValidationError::BillWasRecoursedToTheEnd => {
            err_400(e, JsErrorType::BillWasRecoursedToTheEnd)
        }
        ProtocolValidationError::BillWasNotOfferedToSell => {
            err_400(e, JsErrorType::BillWasNotOfferedToSell)
        }
        ProtocolValidationError::BillWasNotRequestedToPay => {
            err_400(e, JsErrorType::BillWasNotRequestedToPay)
        }
        ProtocolValidationError::BillWasNotRequestedToRecourse => {
            err_400(e, JsErrorType::BillWasNotRequestedToRecourse)
        }
        ProtocolValidationError::BillIsNotOfferToSellWaitingForPayment => {
            err_400(e, JsErrorType::BillIsNotOfferToSellWaitingForPayment)
        }
        ProtocolValidationError::BillIsOfferedToSellAndWaitingForPayment => {
            err_400(e, JsErrorType::BillIsOfferedToSellAndWaitingForPayment)
        }
        ProtocolValidationError::BillIsInRecourseAndWaitingForPayment => {
            err_400(e, JsErrorType::BillIsInRecourseAndWaitingForPayment)
        }
        ProtocolValidationError::BillRequestToAcceptDidNotExpireAndWasNotRejected => err_400(
            e,
            JsErrorType::BillRequestToAcceptDidNotExpireAndWasNotRejected,
        ),
        ProtocolValidationError::BillRequestToPayDidNotExpireAndWasNotRejected => err_400(
            e,
            JsErrorType::BillRequestToPayDidNotExpireAndWasNotRejected,
        ),
        ProtocolValidationError::BillIsNotRequestedToRecourseAndWaitingForPayment => err_400(
            e,
            JsErrorType::BillIsNotRequestedToRecourseAndWaitingForPayment,
        ),
        ProtocolValidationError::BillSellDataInvalid => {
            err_400(e, JsErrorType::BillSellDataInvalid)
        }
        ProtocolValidationError::BillAlreadyPaid => err_400(e, JsErrorType::BillAlreadyPaid),
        ProtocolValidationError::BillNotAccepted => err_400(e, JsErrorType::BillNotAccepted),
        ProtocolValidationError::BillAlreadyRequestedToAccept => {
            err_400(e, JsErrorType::BillAlreadyRequestedToAccept)
        }
        ProtocolValidationError::BillIsRequestedToPayAndWaitingForPayment => {
            err_400(e, JsErrorType::BillIsRequestedToPayAndWaitingForPayment)
        }
        ProtocolValidationError::BillRecourseDataInvalid => {
            err_400(e, JsErrorType::BillRecourseDataInvalid)
        }
        ProtocolValidationError::RecourseeNotPastHolder => {
            err_400(e, JsErrorType::RecourseeNotPastHolder)
        }
        ProtocolValidationError::CallerIsNotDrawee => err_400(e, JsErrorType::CallerIsNotDrawee),
        ProtocolValidationError::CallerIsNotBuyer => err_400(e, JsErrorType::CallerIsNotBuyer),
        ProtocolValidationError::CallerIsNotRecoursee => {
            err_400(e, JsErrorType::CallerIsNotRecoursee)
        }
        ProtocolValidationError::RequestAlreadyRejected => {
            err_400(e, JsErrorType::RequestAlreadyRejected)
        }
        ProtocolValidationError::CallerIsNotHolder => err_400(e, JsErrorType::CallerIsNotHolder),
        ProtocolValidationError::SignatoryAlreadySignatory(_) => {
            err_400(e, JsErrorType::SignatoryAlreadySignatory)
        }
        ProtocolValidationError::CantRemoveLastSignatory => {
            err_400(e, JsErrorType::CantRemoveLastSignatory)
        }
        ProtocolValidationError::NotASignatory(_) => err_400(e, JsErrorType::NotASignatory),
        ProtocolValidationError::NotARemovedOrRejectedSignatory => {
            err_400(e, JsErrorType::NotARemovedOrRejectedSignatory)
        }
        ProtocolValidationError::NotInvitedAsSignatory => {
            err_400(e, JsErrorType::NotInvitedAsSignatory)
        }
        ProtocolValidationError::NoSignerIdentityProof => {
            err_400(e, JsErrorType::NoSignerIdentityProof)
        }
        ProtocolValidationError::FileIsTooBig(_) => err_400(e, JsErrorType::FileIsTooBig),
        ProtocolValidationError::FileIsEmpty => err_400(e, JsErrorType::FileIsEmpty),
        ProtocolValidationError::TooManyFiles => err_400(e, JsErrorType::TooManyFiles),
        ProtocolValidationError::InvalidFileName(_) => err_400(e, JsErrorType::InvalidFileName),
        ProtocolValidationError::Blockchain(e) => err_500(e, JsErrorType::Blockchain),
        ProtocolValidationError::InvalidRelayUrl => err_400(e, JsErrorType::InvalidRelayUrl),
        ProtocolValidationError::InvalidSignature => err_400(e, JsErrorType::InvalidSignature),
        ProtocolValidationError::InvalidHash => err_400(e, JsErrorType::InvalidHash),
        ProtocolValidationError::InvalidUrl => err_400(e, JsErrorType::InvalidUrl),
        ProtocolValidationError::InvalidBillAction => err_400(e, JsErrorType::InvalidBillAction),
        ProtocolValidationError::InvalidCompanyAction => {
            err_400(e, JsErrorType::InvalidCompanyAction)
        }
        ProtocolValidationError::InvalidIdentityAction => {
            err_400(e, JsErrorType::InvalidIdentityAction)
        }
        ProtocolValidationError::CompanySignerCreatorMismatch => {
            err_400(e, JsErrorType::CompanySignerCreatorMismatch)
        }
        ProtocolValidationError::InvalidMintRequestId => {
            err_400(e, JsErrorType::InvalidMintRequestId)
        }
        ProtocolValidationError::CallerMustBeSignatory => {
            err_400(e, JsErrorType::CallerMustBeSignatory)
        }
        ProtocolValidationError::CallerMustBeCreator => {
            err_400(e, JsErrorType::CallerMustBeCreator)
        }
        ProtocolValidationError::CallerMustBeIdentifiedCreator => {
            err_400(e, JsErrorType::CallerMustBeIdentifiedCreator)
        }
        ProtocolValidationError::InvalidIdentityProof => {
            err_400(e, JsErrorType::InvalidIdentityProof)
        }
        ProtocolValidationError::InvalidReferenceBlock => {
            err_400(e, JsErrorType::InvalidReferenceBlock)
        }
        ProtocolValidationError::MintRequestToPayWithoutCustomAddress => {
            err_400(e, JsErrorType::MintRequestToPayWithoutCustomAddress)
        }
    }
}

fn err_400<E: ToString>(e: E, t: JsErrorType) -> JsErrorData {
    JsErrorData {
        error: t,
        message: e.to_string(),
        code: 400,
    }
}

fn err_404<E: ToString>(e: E, t: JsErrorType) -> JsErrorData {
    JsErrorData {
        error: t,
        message: e.to_string(),
        code: 404,
    }
}

fn err_500<E: ToString>(e: E, t: JsErrorType) -> JsErrorData {
    JsErrorData {
        error: t,
        message: e.to_string(),
        code: 500,
    }
}
