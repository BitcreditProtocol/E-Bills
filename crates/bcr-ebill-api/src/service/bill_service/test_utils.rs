use super::*;
use crate::{
    external::{self, court::MockCourtClientApi, file_storage::MockFileStorageClientApi},
    service::{
        company_service::tests::{get_valid_company_chain, get_valid_identity_chain},
        contact_service::tests::get_baseline_contact,
        transport_service::{MockNotificationTransportServiceApi, MockTransportServiceApi},
    },
    tests::tests::{
        MockBillChainStoreApiMock, MockBillStoreApiMock, MockCompanyChainStoreApiMock,
        MockCompanyStoreApiMock, MockContactStoreApiMock, MockFileReferenceStoreApiMock,
        MockFileUploadStoreApiMock, MockIdentityChainStoreApiMock, MockIdentityStoreApiMock,
        MockMintStore, MockNostrContactStore, bill_id_test,
        bill_identified_participant_only_node_id, bill_participant_only_node_id, empty_address,
        empty_bill_identified_participant, empty_bitcredit_bill, empty_identity,
        empty_other_identity, init_test_cfg, node_id_test, node_id_test_other, node_id_test_other2,
        private_key_test, private_key_test_another, signed_identity_proof_test, test_ts,
        valid_payment_address_testnet,
    },
};
use bcr_ebill_core::{
    application::bill::{
        BillAcceptState, BillAcceptanceStatus, BillCallerActions, BillData, BillMintState,
        BillMintStatus, BillParticipants, BillPaymentState, BillPaymentStatus, BillRecourseStatus,
        BillSellStatus, BillState, BillStatus, PaidData, PaymentState,
    },
    protocol::{
        Address, City, Country, Date, Name, Sum, Timestamp,
        blockchain::bill::{
            BillBlock,
            block::{
                BillAcceptBlockData, BillIssueBlockData, BillOfferToSellBlockData,
                BillParticipantBlockData, BillPaymentBlockData, BillRecourseBlockData,
                BillRecourseReasonBlockData, BillRejectBlockData, BillRejectToBuyBlockData,
                BillRequestRecourseBlockData, BillRequestToAcceptBlockData,
                BillRequestToPayBlockData, BillSellBlockData,
            },
            participant::{BillIdentParticipant, BillParticipant},
        },
        constants::{
            ACCEPT_DEADLINE_SECONDS, DAY_IN_SECS, PAYMENT_DEADLINE_SECONDS,
            RECOURSE_DEADLINE_SECONDS,
        },
        file_reference::FileReference,
    },
};
use external::{bitcoin::MockBitcoinClientApi, mint::MockMintClientApi};
use service::BillService;
use std::{collections::HashMap, sync::Arc};

pub struct MockBillContext {
    pub contact_store: MockContactStoreApiMock,
    pub bill_store: MockBillStoreApiMock,
    pub bill_blockchain_store: MockBillChainStoreApiMock,
    pub identity_store: MockIdentityStoreApiMock,
    pub identity_chain_store: MockIdentityChainStoreApiMock,
    pub company_chain_store: MockCompanyChainStoreApiMock,
    pub company_store: MockCompanyStoreApiMock,
    pub file_upload_store: MockFileUploadStoreApiMock,
    pub file_upload_client: MockFileStorageClientApi,
    pub file_reference_store: MockFileReferenceStoreApiMock,
    pub transport_service: MockTransportServiceApi,
    pub mint_store: MockMintStore,
    pub mint_client: MockMintClientApi,
    pub court_client: MockCourtClientApi,
    pub nostr_contact_store: MockNostrContactStore,
}

pub fn get_baseline_identity() -> IdentityWithAll {
    let keys = BcrKeys::from_private_key(&private_key_test());
    let mut identity = empty_identity();
    identity.name = Name::new("drawer").unwrap();
    identity.node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
    identity.postal_address.country = Some(Country::AT);
    identity.postal_address.city = Some(City::new("Vienna").unwrap());
    identity.postal_address.address = Some(Address::new("Hayekweg 5").unwrap());
    identity.nostr_relays = vec![url::Url::parse("ws://localhost:8080").unwrap()];
    IdentityWithAll {
        identity,
        key_pair: keys,
    }
}

pub fn get_baseline_cached_bill(id: BillId) -> BitcreditBillResult {
    BitcreditBillResult {
        id,
        participants: BillParticipants {
            drawee: bill_identified_participant_only_node_id(node_id_test()),
            drawer: bill_identified_participant_only_node_id(node_id_test_other()),
            payee: BillParticipant::Ident(bill_identified_participant_only_node_id(
                node_id_test_other2(),
            )),
            endorsee: None,
            endorsements: vec![],
            endorsements_count: 5,
            all_participant_node_ids: vec![
                node_id_test(),
                node_id_test_other(),
                node_id_test_other2(),
            ],
        },
        data: BillData {
            time_of_drawing: test_ts(),
            issue_date: Date::new("2024-05-01").unwrap(),
            time_of_maturity: test_ts(),
            maturity_date: Date::new("2024-07-01").unwrap(),
            country_of_issuing: Country::AT,
            city_of_issuing: City::new("Vienna").unwrap(),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            sum: Sum::new_sat(15000).expect("sat works"),
            files: vec![],
            active_notification: None,
        },
        status: BillStatus {
            acceptance: BillAcceptanceStatus {
                time_of_request_to_accept: None,
                requested_to_accept: false,
                accepted: false,
                request_to_accept_timed_out: false,
                rejected_to_accept: false,
                acceptance_deadline_timestamp: None,
            },
            payment: BillPaymentStatus {
                time_of_request_to_pay: None,
                requested_to_pay: false,
                paid: false,
                request_to_pay_timed_out: false,
                rejected_to_pay: false,
                payment_deadline_timestamp: None,
            },
            sell: BillSellStatus {
                time_of_last_offer_to_sell: None,
                sold: false,
                offered_to_sell: false,
                offer_to_sell_timed_out: false,
                rejected_offer_to_sell: false,
                buying_deadline_timestamp: None,
            },
            recourse: BillRecourseStatus {
                time_of_last_request_to_recourse: None,
                recoursed: false,
                requested_to_recourse: false,
                request_to_recourse_timed_out: false,
                rejected_request_to_recourse: false,
                recourse_deadline_timestamp: None,
            },
            mint: BillMintStatus {
                has_mint_requests: false,
            },
            redeemed_funds_available: false,
            has_requested_funds: false,
            last_block_time: test_ts(),
            is_mature: false,
        },
        state: BillState {
            mint: BillMintState::None,
            accept: BillAcceptState::None,
            payment: BillPaymentState::None,
        },
        current_waiting_state: None,
        history: BillHistory { blocks: vec![] },
        actions: BillCallerActions {
            bill_actions: vec![],
            payment_actions: vec![],
        },
    }
}

pub fn get_baseline_bill(bill_id: &BillId) -> BitcreditBill {
    let mut bill = empty_bitcredit_bill();
    let keys = BcrKeys::new();

    bill.maturity_date = Date::new("2099-10-15").unwrap();
    let mut payee = empty_bill_identified_participant();
    payee.name = Name::new("payee").unwrap();
    payee.node_id = NodeId::new(keys.pub_key(), bitcoin::Network::Testnet);
    bill.payee = BillParticipant::Ident(payee);
    bill.drawee = BillIdentParticipant::new(get_baseline_identity().identity).unwrap();
    bill.id = bill_id.to_owned();
    bill
}

pub fn get_genesis_chain(bill: Option<BitcreditBill>) -> BillBlockchain {
    let bill = bill.unwrap_or(get_baseline_bill(&bill_id_test()));
    BillBlockchain::new(
        &BillIssueBlockData::from(bill, None, test_ts() - 15, signed_identity_proof_test()),
        get_baseline_identity().key_pair,
        None,
        BcrKeys::from_private_key(&private_key_test()),
        test_ts() - 15,
    )
    .unwrap()
}

pub fn get_service(mut ctx: MockBillContext) -> BillService {
    init_test_cfg();
    let mut bitcoin_client = MockBitcoinClientApi::new();
    bitcoin_client
        .expect_check_payment_for_address()
        .returning(|_, _| {
            Ok(PaymentState::PaidConfirmed(PaidData {
                block_time: test_ts(),
                block_hash: "000000000061ad7b0d52af77e5a9dbcdc421bf00e93992259f16b2cf2693c4b1"
                    .into(),
                confirmations: 7,
                tx_id: "80e4dc03b2ea934c97e265fa1855eba5c02788cb269e3f43a8e9a7bb0e114e2c".into(),
            }))
        });
    ctx.nostr_contact_store
        .expect_by_node_id()
        .returning(|_| Ok(None));
    ctx.contact_store.expect_get().returning(|node_id| {
        let mut contact = get_baseline_contact();
        contact.node_id = node_id.to_owned();
        Ok(Some(contact))
    });
    ctx.contact_store
        .expect_get_map()
        .returning(|| Ok(HashMap::new()));
    ctx.identity_chain_store
        .expect_add_block()
        .returning(|_| Ok(()));
    ctx.company_chain_store
        .expect_add_block()
        .returning(|_, _| Ok(()));
    ctx.company_chain_store
        .expect_get_chain()
        .returning(|_| Ok(get_valid_company_chain()));
    ctx.identity_chain_store
        .expect_get_chain()
        .returning(|| Ok(get_valid_identity_chain()));
    ctx.bill_blockchain_store
        .expect_add_block()
        .returning(|_, _| Ok(()));
    ctx.bill_store
        .expect_get_keys()
        .returning(|_| Ok(BcrKeys::from_private_key(&private_key_test())));
    let payment_state_paid = PaymentState::PaidConfirmed(PaidData {
        block_time: test_ts(),
        block_hash: "000000000061ad7b0d52af77e5a9dbcdc421bf00e93992259f16b2cf2693c4b1".into(),
        confirmations: 7,
        tx_id: "80e4dc03b2ea934c97e265fa1855eba5c02788cb269e3f43a8e9a7bb0e114e2c".into(),
    });
    let payment_state_clone = payment_state_paid.clone();
    let payment_state_clone2 = payment_state_paid.clone();
    ctx.bill_store
        .expect_get_payment_state()
        .returning(move |_| Ok(Some(payment_state_clone.clone())));
    ctx.bill_store
        .expect_get_offer_to_sell_payment_state()
        .returning(move |_, _| Ok(Some(payment_state_clone2.clone())));
    ctx.bill_store
        .expect_get_recourse_payment_state()
        .returning(move |_, _| Ok(Some(payment_state_paid.clone())));
    ctx.bill_store
        .expect_get_bill_from_cache()
        .returning(|_, _| Ok(None));
    ctx.bill_store
        .expect_get_bills_from_cache()
        .returning(|_, _| Ok(vec![]));
    ctx.bill_store
        .expect_invalidate_bill_in_cache()
        .returning(|_| Ok(()));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.identity_store
        .expect_get()
        .returning(|| Ok(empty_other_identity()));
    ctx.identity_store.expect_get_full().returning(|| {
        Ok(IdentityWithAll {
            identity: empty_other_identity(),
            key_pair: BcrKeys::from_private_key(&private_key_test_another()),
        })
    });
    ctx.mint_store
        .expect_exists_for_bill()
        .returning(|_, _| Ok(false));
    ctx.file_reference_store
        .expect_upsert()
        .returning(|hash, nostr_hash, name, _, _, _| {
            Ok(FileReference::new(hash.clone(), *nostr_hash, name))
        });
    ctx.file_reference_store
        .expect_upsert()
        .returning(|hash, nostr_hash, name, _, _, _| {
            Ok(FileReference::new(hash.clone(), *nostr_hash, name))
        });
    ctx.file_reference_store
        .expect_get()
        .returning(|_| Ok(None));
    ctx.file_reference_store
        .expect_add_server_urls()
        .returning(|_, _| Ok(true));
    ctx.transport_service
        .expect_publish_file_metadata()
        .returning(|_, _, _, _, _| Ok(()));
    let mut default_notification = MockNotificationTransportServiceApi::new();
    default_notification
        .expect_get_active_bill_notification()
        .returning(|_| None);
    default_notification
        .expect_create_local_bill_notification()
        .returning(|_, _, _, _, _| Ok(()));
    default_notification
        .expect_reconcile_quote_applicant_action_notification()
        .returning(|_, _, _, _| Ok(()));
    default_notification
        .expect_create_general_notification()
        .returning(|_, _, _, _| Ok(()));
    ctx.transport_service
        .expect_notification_transport()
        .times(0..)
        .return_const(Arc::new(default_notification));
    BillService::new(
        Arc::new(ctx.bill_store),
        Arc::new(ctx.bill_blockchain_store),
        Arc::new(ctx.identity_store),
        Arc::new(ctx.file_upload_store),
        Arc::new(ctx.file_upload_client),
        Arc::new(ctx.file_reference_store),
        Arc::new(bitcoin_client),
        Arc::new(ctx.transport_service),
        Arc::new(ctx.identity_chain_store),
        Arc::new(ctx.company_chain_store),
        Arc::new(ctx.contact_store),
        Arc::new(ctx.company_store),
        Arc::new(ctx.mint_store),
        Arc::new(ctx.mint_client),
        Arc::new(ctx.court_client),
        Arc::new(ctx.nostr_contact_store),
    )
}

pub fn get_ctx() -> MockBillContext {
    MockBillContext {
        bill_store: MockBillStoreApiMock::new(),
        bill_blockchain_store: MockBillChainStoreApiMock::new(),
        identity_store: MockIdentityStoreApiMock::new(),
        file_upload_store: MockFileUploadStoreApiMock::new(),
        file_upload_client: MockFileStorageClientApi::new(),
        file_reference_store: MockFileReferenceStoreApiMock::new(),
        identity_chain_store: MockIdentityChainStoreApiMock::new(),
        company_chain_store: MockCompanyChainStoreApiMock::new(),
        contact_store: MockContactStoreApiMock::new(),
        company_store: MockCompanyStoreApiMock::new(),
        transport_service: MockTransportServiceApi::new(),
        mint_store: MockMintStore::new(),
        mint_client: MockMintClientApi::new(),
        court_client: MockCourtClientApi::new(),
        nostr_contact_store: MockNostrContactStore::new(),
    }
}

pub fn request_to_recourse_block(
    id: &BillId,
    first_block: &BillBlock,
    recoursee: &BillIdentParticipant,
    ts: Option<Timestamp>,
) -> BillBlock {
    let timestamp = ts.unwrap_or(first_block.timestamp + 1);
    BillBlock::create_block_for_request_recourse(
        id.to_owned(),
        first_block,
        &BillRequestRecourseBlockData {
            recourser: BillParticipant::Ident(bill_identified_participant_only_node_id(
                node_id_test(),
            ))
            .into(),
            recoursee: recoursee.to_owned().into(),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(15000).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: timestamp + 2 * RECOURSE_DEADLINE_SECONDS,
            },
            recourse_reason: BillRecourseReasonBlockData::Pay,
            signatory: None,
            signing_timestamp: timestamp,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        timestamp,
    )
    .expect("block could not be created")
}

pub fn recourse_block(
    id: &BillId,
    first_block: &BillBlock,
    recoursee: &BillIdentParticipant,
) -> BillBlock {
    BillBlock::create_block_for_recourse(
        id.to_owned(),
        first_block,
        &BillRecourseBlockData {
            recourser: BillParticipant::Ident(bill_identified_participant_only_node_id(
                node_id_test(),
            ))
            .into(),
            recoursee: recoursee.to_owned().into(),
            signatory: None,
            signing_timestamp: first_block.timestamp + 1,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp + 1,
    )
    .expect("block could not be created")
}

pub fn reject_recourse_block(id: &BillId, first_block: &BillBlock) -> BillBlock {
    BillBlock::create_block_for_reject_to_pay_recourse(
        id.to_owned(),
        first_block,
        &BillRejectBlockData {
            rejecter: bill_identified_participant_only_node_id(node_id_test()).into(),
            signatory: None,
            signing_timestamp: first_block.timestamp,
            signing_address: empty_address(),
            signer_identity_proof: signed_identity_proof_test().into(),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp,
    )
    .expect("block could not be created")
}

pub fn request_to_accept_block(
    id: &BillId,
    first_block: &BillBlock,
    ts: Option<Timestamp>,
) -> BillBlock {
    let timestamp = ts.unwrap_or(first_block.timestamp + 1);
    BillBlock::create_block_for_request_to_accept(
        id.to_owned(),
        first_block,
        &BillRequestToAcceptBlockData {
            requester: BillParticipantBlockData::Ident(
                bill_identified_participant_only_node_id(node_id_test()).into(),
            ),
            signatory: None,
            signing_timestamp: timestamp,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
            acceptance_deadline_timestamp: timestamp + 2 * ACCEPT_DEADLINE_SECONDS,
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        timestamp,
    )
    .expect("block could not be created")
}

pub fn reject_accept_block(id: &BillId, first_block: &BillBlock) -> BillBlock {
    BillBlock::create_block_for_reject_to_accept(
        id.to_owned(),
        first_block,
        &BillRejectBlockData {
            rejecter: bill_identified_participant_only_node_id(node_id_test()).into(),
            signatory: None,
            signing_timestamp: first_block.timestamp,
            signing_address: empty_address(),
            signer_identity_proof: signed_identity_proof_test().into(),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp,
    )
    .expect("block could not be created")
}

pub fn offer_to_sell_block(
    id: &BillId,
    first_block: &BillBlock,
    buyer: &BillIdentParticipant,
    ts: Option<Timestamp>,
) -> BillBlock {
    let timestamp = ts.unwrap_or(first_block.timestamp + 1);
    BillBlock::create_block_for_offer_to_sell(
        id.to_owned(),
        first_block,
        &BillOfferToSellBlockData {
            seller: BillParticipantBlockData::Ident(
                bill_identified_participant_only_node_id(node_id_test()).into(),
            ),
            buyer: BillParticipantBlockData::Ident(buyer.to_owned().into()),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(15000).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: timestamp + 2 * DAY_IN_SECS,
            },
            signatory: None,
            signing_timestamp: timestamp,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        timestamp,
    )
    .expect("block could not be created")
}

pub fn reject_buy_block(id: &BillId, first_block: &BillBlock) -> BillBlock {
    BillBlock::create_block_for_reject_to_buy(
        id.to_owned(),
        first_block,
        &BillRejectToBuyBlockData {
            rejecter: bill_participant_only_node_id(node_id_test()).into(),
            signatory: None,
            signing_timestamp: first_block.timestamp,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp,
    )
    .expect("block could not be created")
}

pub fn sell_block(id: &BillId, first_block: &BillBlock, buyer: &BillIdentParticipant) -> BillBlock {
    BillBlock::create_block_for_sell(
        id.to_owned(),
        first_block,
        &BillSellBlockData {
            seller: BillParticipantBlockData::Ident(
                bill_identified_participant_only_node_id(node_id_test()).into(),
            ),
            buyer: BillParticipantBlockData::Ident(buyer.to_owned().into()),
            signatory: None,
            signing_timestamp: first_block.timestamp + 1,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp + 1,
    )
    .expect("block could not be created")
}

pub fn accept_block(id: &BillId, first_block: &BillBlock) -> BillBlock {
    BillBlock::create_block_for_accept(
        id.to_owned(),
        first_block,
        &BillAcceptBlockData {
            accepter: bill_identified_participant_only_node_id(node_id_test()).into(),
            signatory: None,
            signing_timestamp: first_block.timestamp + 1,
            signing_address: empty_address(),
            signer_identity_proof: signed_identity_proof_test().into(),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp + 1,
    )
    .expect("block could not be created")
}

pub fn request_to_pay_block(
    id: &BillId,
    first_block: &BillBlock,
    ts: Option<Timestamp>,
) -> BillBlock {
    let timestamp = ts.unwrap_or(first_block.timestamp + 1);
    BillBlock::create_block_for_request_to_pay(
        id.to_owned(),
        first_block,
        &BillRequestToPayBlockData {
            requester: BillParticipantBlockData::Ident(
                bill_identified_participant_only_node_id(node_id_test()).into(),
            ),
            payment_data: BillPaymentBlockData {
                sum: Sum::new_sat(15000).expect("sat works"),
                payment_address: valid_payment_address_testnet(),
                payment_deadline: timestamp + 2 * PAYMENT_DEADLINE_SECONDS,
            },
            signatory: None,
            signing_timestamp: timestamp,
            signing_address: Some(empty_address()),
            signer_identity_proof: Some(signed_identity_proof_test().into()),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        timestamp,
    )
    .expect("block could not be created")
}

pub fn reject_to_pay_block(id: &BillId, first_block: &BillBlock) -> BillBlock {
    BillBlock::create_block_for_reject_to_pay(
        id.to_owned(),
        first_block,
        &BillRejectBlockData {
            rejecter: bill_identified_participant_only_node_id(node_id_test()).into(),
            signatory: None,
            signing_timestamp: first_block.timestamp + 1,
            signing_address: empty_address(),
            signer_identity_proof: signed_identity_proof_test().into(),
        },
        &BcrKeys::from_private_key(&private_key_test()),
        None,
        &BcrKeys::from_private_key(&private_key_test()),
        first_block.timestamp + 1,
    )
    .expect("block could not be created")
}

pub fn bill_keys() -> BcrKeys {
    BcrKeys::from_private_key(&private_key_test())
}

pub fn safe_deadline_ts(min_deadline: u64) -> Timestamp {
    Timestamp::now() + 2 * min_deadline
}
