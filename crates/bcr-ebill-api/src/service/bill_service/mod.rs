use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::application::bill::{
    AddressDerivationMetadataForPaymentRequest, BillCombinedBitcoinKey, BillsBalanceOverview,
    BillsFilterRole, BitcreditBillResult, Endorsement, LightBitcreditBillResult, PastPaymentResult,
    SweepEstimate, SweepResult,
};
use bcr_ebill_core::application::identity::{Identity, IdentityWithAll};
use bcr_ebill_core::protocol::BitcoinAddress;
use bcr_ebill_core::protocol::Currency;
use bcr_ebill_core::protocol::File;
use bcr_ebill_core::protocol::SecretKey;
use bcr_ebill_core::protocol::Sum;
use bcr_ebill_core::protocol::Timestamp;
use bcr_ebill_core::protocol::blockchain::bill::chain::BillBlockPlaintextWrapper;
use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
use bcr_ebill_core::protocol::blockchain::bill::{
    BillAction, BillBlockchain, BillHistory, BillIssueData, BitcreditBill,
    participant::PastEndorsee,
};
use bcr_ebill_core::protocol::crypto::BcrKeys;
use bcr_ebill_core::protocol::mint::MintRequestState;

use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

/// Generic result type
pub type Result<T> = std::result::Result<T, error::Error>;
pub use error::Error;
pub use service::BillService;

mod blocks;
mod data_fetching;
mod error;
mod issue;
mod payment;
mod propagation;
mod service;
#[cfg(test)]
pub mod test_utils;
#[cfg(test)]
pub mod tests;

#[cfg(test)]
impl ServiceTraitBounds for MockBillServiceApi {}

#[cfg_attr(test, automock)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait BillServiceApi: ServiceTraitBounds {
    /// Get bill balances
    async fn get_bill_balances(
        &self,
        currency: &Currency,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<BillsBalanceOverview>;

    /// Search for bills
    async fn search_bills(
        &self,
        currency: &Currency,
        search_term: &Option<String>,
        date_range_from: Option<Timestamp>,
        date_range_to: Option<Timestamp>,
        role: &BillsFilterRole,
        participants: &[NodeId],
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<LightBitcreditBillResult>>;

    /// Gets all bills
    async fn get_bills(
        &self,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<BitcreditBillResult>>;

    /// Gets the combined bitcoin private keys for a given bill
    async fn get_combined_bitcoin_keys_for_bill(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
    ) -> Result<Vec<BillCombinedBitcoinKey>>;

    /// Gets the detail for the given bill id
    async fn get_detail(
        &self,
        bill_id: &BillId,
        local_identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<BitcreditBillResult>;

    /// Gets the keys for a given bill
    async fn get_bill_keys(&self, bill_id: &BillId) -> Result<BcrKeys>;

    /// opens and decrypts the attached file from the given bill
    async fn open_and_decrypt_attached_file(
        &self,
        bill_id: &BillId,
        file: &File,
        bill_private_key: &SecretKey,
    ) -> Result<Vec<u8>>;

    /// issues a new bill
    async fn issue_new_bill(&self, data: BillIssueData) -> Result<BitcreditBill>;

    /// executes the given bill action
    async fn execute_bill_action(
        &self,
        bill_id: &BillId,
        bill_action: BillAction,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<BillBlockchain>;

    /// Check payment status of bills that are requested to pay and not expired and not paid yet, updating their
    /// paid status if they were paid
    async fn check_bills_payment(&self) -> Result<()>;

    /// Check payment status of the bill with the given id that is requested to pay and not expired
    /// and not paid yet updating their paid status if they were paid
    async fn check_payment_for_bill(&self, bill_id: &BillId, identity: &Identity) -> Result<()>;

    /// Check payment status of bills that are waiting for a payment on an OfferToSell block, which
    /// haven't been expired, adding a Sell block if they were paid
    async fn check_bills_offer_to_sell_payment(&self) -> Result<()>;

    /// Check payment status of the bill with the given id that is waiting for a payment on an OfferToSell block, which
    /// haven't been expired, adding a Sell block if they were paid
    async fn check_offer_to_sell_payment_for_bill(
        &self,
        bill_id: &BillId,
        identity: &IdentityWithAll,
    ) -> Result<()>;

    /// Check payment status of bills that are waiting for a payment on an RequestRecourse block, which
    /// haven't been expired, adding a Recourse block if they were paid
    async fn check_bills_in_recourse_payment(&self) -> Result<()>;

    /// Check payment status of the bill with the given id that is waiting for a payment on an
    /// RequestRecourse block, which haven't been expired, adding a Recourse block if they were paid
    async fn check_recourse_payment_for_bill(
        &self,
        bill_id: &BillId,
        identity: &IdentityWithAll,
    ) -> Result<()>;

    /// Check if actions expected on bills in certain states have expired and execute the necessary
    /// steps after timeout.
    async fn check_bills_timeouts(&self, now: Timestamp) -> Result<()>;

    /// Returns previous endorseers of the bill to select from for Recourse
    async fn get_past_endorsees(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<PastEndorsee>>;

    /// Returns previous payment requests of the given bill, where the user with the given node id
    /// was the financial beneficiary, with the metadata and outcomes
    async fn get_past_payments(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<Vec<PastPaymentResult>>;

    /// Returns all endorsements of the bill
    async fn get_endorsements(
        &self,
        bill_id: &BillId,
        identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<Vec<Endorsement>>;

    /// Clear the bill cache
    async fn clear_bill_cache(&self) -> Result<()>;

    /// request to mint a bill
    async fn request_to_mint(
        &self,
        bill_id: &BillId,
        mint_node_id: &NodeId,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// replace a terminal mint quote after AI Credit returned signed reviewed-correction authority
    async fn request_to_mint_reissue(
        &self,
        bill_id: &BillId,
        mint_node_id: &NodeId,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
        signed_reissue_permit_json: &str,
    ) -> Result<()>;

    /// Returns the mint state for a given bill
    async fn get_mint_state(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<MintRequestState>>;

    /// Cancel a pending request to mint
    async fn cancel_request_to_mint(
        &self,
        mint_request_id: &Uuid,
        current_identity_node_id: &NodeId,
    ) -> Result<()>;

    /// Accept a mint offer for a given request to mint
    async fn accept_mint_offer(
        &self,
        mint_request_id: &Uuid,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        timestamp: Timestamp,
    ) -> Result<()>;

    /// Reject a mint offer for a given request to mint
    async fn reject_mint_offer(
        &self,
        mint_request_id: &Uuid,
        current_identity_node_id: &NodeId,
    ) -> Result<()>;

    /// Check mint state for a given bill
    async fn check_mint_state(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<()>;

    /// Check mint state for all bills
    async fn check_mint_state_for_all_bills(&self) -> Result<()>;

    /// If dev mode is on, return the full bill chain with decrypted data
    async fn dev_mode_get_full_bill_chain(
        &self,
        bill_id: &BillId,
        current_identity_node_id: &NodeId,
    ) -> Result<Vec<BillBlockPlaintextWrapper>>;

    /// Shares a bill with the configured court and the given court node id
    async fn share_bill_with_court(
        &self,
        bill_id: &BillId,
        signer_public_data: &BillParticipant,
        signer_keys: &BcrKeys,
        court_node_id: &NodeId,
    ) -> Result<()>;

    /// Return a bill's history
    async fn get_bill_history(
        &self,
        bill_id: &BillId,
        local_identity: &Identity,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        current_timestamp: Timestamp,
    ) -> Result<BillHistory>;

    /// Return a bill's Blockchain
    async fn get_local_bill_chain(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
    ) -> Result<BillBlockchain>;

    fn mempool_link(&self, address: &BitcoinAddress) -> String;

    fn link_to_pay(&self, address: &BitcoinAddress, sum: &Sum, bill_id: &BillId) -> String;

    async fn check_and_estimate_btc_sweep(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        source_address: &BitcoinAddress,
        destination_address: &BitcoinAddress,
    ) -> Result<SweepEstimate>;

    async fn btc_sweep(
        &self,
        bill_id: &BillId,
        caller_public_data: &BillParticipant,
        caller_keys: &BcrKeys,
        source_address: &BitcoinAddress,
        destination_address: &BitcoinAddress,
        fee: u64,
    ) -> Result<SweepResult>;

    /// Calculates and returns the next block id and last block hash for
    /// address derivation in payment requests for mint
    async fn get_address_derivation_metadata_for_payment_request(
        &self,
        bill_id: &BillId,
    ) -> Result<AddressDerivationMetadataForPaymentRequest>;
}
