use super::*;
use crate::{
    external::mint::QuoteStatusReply,
    service::{
        bill_service::test_utils::{MockBillContext, safe_deadline_ts},
        company_service::tests::{
            get_baseline_company, get_baseline_company_data, get_valid_company_chain,
        },
    },
    tests::tests::{
        bill_id_test, bill_id_test_other, bill_id_test_other2,
        bill_identified_participant_only_node_id, bill_participant_only_node_id, empty_address,
        empty_bill_identified_participant, empty_identity, init_test_cfg, node_id_test,
        node_id_test_another, node_id_test_other, private_key_test, private_key_test_another,
        signed_identity_proof_test, signed_other_identity_proof_test, test_ts,
        valid_payment_address_testnet,
    },
    util::get_uuid_v4,
};
use bcr_common::{cashu::nut02 as cdk02, ecash};
use bcr_ebill_core::{
    application::{
        ValidationError,
        bill::{
            BillAcceptanceStatus, BillCallerBillAction, BillCurrentWaitingState, BillPaymentStatus,
            BillRecourseStatus, BillSellStatus, BillWaitingForPaymentState,
            BillWaitingStatePaymentData,
        },
    },
    protocol::{
        Address, BlockId, City, Country, Currency, Date, DateTimeUtc, File, Name,
        ProtocolValidationError, Sha256Hash, Sum,
        blockchain::{
            Block, Blockchain,
            bill::{
                BillBlock, BillOpCode, ContactType, PaymentStatus, RecourseReason,
                block::{
                    BillEndorseBlockData, BillMintBlockData, BillOfferToSellBlockData,
                    BillParticipantBlockData, BillPaymentBlockData, BillRecourseReasonBlockData,
                    BillRejectBlockData, BillRequestRecourseBlockData,
                    BillRequestToAcceptBlockData, BillRequestToPayBlockData, BillSellBlockData,
                    BillSignatoryBlockData,
                },
                participant::{BillAnonParticipant, BillIdentParticipant, BillParticipant},
            },
        },
        constants::{
            ACCEPT_DEADLINE_SECONDS, DAY_IN_SECS, PAYMENT_DEADLINE_SECONDS,
            RECOURSE_DEADLINE_SECONDS,
        },
        event::ActionType,
        mint::{MintOffer, MintRequest, MintRequestStatus},
    },
};
use bitcoin::hashes::sha256::Hash as Sha256HexHash;
use mockall::predicate::{always, eq, function};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use test_utils::{
    accept_block, get_baseline_bill, get_baseline_cached_bill, get_baseline_identity, get_ctx,
    get_genesis_chain, get_service, offer_to_sell_block, recourse_block, reject_accept_block,
    reject_buy_block, reject_recourse_block, reject_to_pay_block, request_to_accept_block,
    request_to_pay_block, request_to_recourse_block, sell_block,
};

#[tokio::test]
async fn get_bill_balances_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let company_keys = BcrKeys::new();
    let company_node_id = NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet);

    let mut bill1 = get_baseline_bill(&bill_id_test());
    bill1.sum = Sum::new_sat(1000).expect("sat works");
    bill1.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let mut bill2 = get_baseline_bill(&bill_id_test_other());
    bill2.sum = Sum::new_sat(2000).expect("sat works");
    bill2.drawee = bill_identified_participant_only_node_id(company_node_id.clone());
    bill2.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    let mut bill3 = get_baseline_bill(&bill_id_test_other2());
    bill3.sum = Sum::new_sat(20000).expect("sat works");
    bill3.drawer = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    bill3.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        company_node_id.clone(),
    ));
    bill3.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));

    ctx.bill_store.expect_get_ids().returning(|| {
        Ok(vec![
            bill_id_test(),
            bill_id_test_other(),
            bill_id_test_other2(),
        ])
    });
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test())
        .returning(move |_| Ok(get_genesis_chain(Some(bill1.clone()))));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test_other())
        .returning(move |_| Ok(get_genesis_chain(Some(bill2.clone()))));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test_other2())
        .returning(move |_| Ok(get_genesis_chain(Some(bill3.clone()))));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));

    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification().returning(|_| None);
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);

    // for identity
    let res = service
        .get_bill_balances(
            &Currency::sat(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().payer.sum,
        Sum::new_sat(1000).expect("sat works")
    );
    assert_eq!(
        res.as_ref().unwrap().payee.sum,
        Sum::new_sat(2000).expect("sat works")
    );
    assert_eq!(
        res.as_ref().unwrap().contingent.sum,
        Sum::new_sat(20000).expect("sat works")
    );

    // for company
    let res_comp = service
        .get_bill_balances(
            &Currency::sat(),
            &BillParticipant::Ident(bill_identified_participant_only_node_id(
                company_node_id.clone(),
            )),
            &company_keys,
        )
        .await;
    assert!(res_comp.is_ok());
    assert_eq!(
        res_comp.as_ref().unwrap().payer.sum,
        Sum::new_sat(2000).expect("sat works")
    );
    assert_eq!(
        res_comp.as_ref().unwrap().payee.sum,
        Sum::new_sat(20000).expect("sat works")
    );
    assert_eq!(
        res_comp.as_ref().unwrap().contingent.sum,
        Sum::new_sat_zero_allowed(0)
    );
}

#[tokio::test]
async fn get_search_bill() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let company_keys = BcrKeys::new();
    let company_node_id = NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet);

    let mut bill1 = get_baseline_bill(&bill_id_test());
    let bill1_payee = bill1.payee.clone();
    bill1.issue_date = Date::new("2020-05-01").unwrap();
    bill1.sum = Sum::new_sat(1000).expect("sat works");
    bill1.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let mut bill2 = get_baseline_bill(&bill_id_test_other());
    bill2.issue_date = Date::new("2030-05-01").unwrap();
    bill2.sum = Sum::new_sat(2000).expect("sat works");
    bill2.drawee = bill_identified_participant_only_node_id(company_node_id.clone());
    let mut payee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    payee.name = Name::new("hayek").unwrap();
    bill2.payee = BillParticipant::Ident(payee.clone());
    let mut bill3 = get_baseline_bill(&bill_id_test_other2());
    bill3.issue_date = Date::new("2030-05-01").unwrap();
    bill3.sum = Sum::new_sat(20000).expect("sat works");
    bill3.drawer = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    bill3.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        company_node_id.clone(),
    ));
    bill3.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));

    ctx.bill_store.expect_get_ids().returning(|| {
        Ok(vec![
            bill_id_test(),
            bill_id_test_other(),
            bill_id_test_other2(),
        ])
    });
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test())
        .returning(move |_| Ok(get_genesis_chain(Some(bill1.clone()))));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test_other())
        .returning(move |_| Ok(get_genesis_chain(Some(bill2.clone()))));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .withf(|id| *id == bill_id_test_other2())
        .returning(move |_| Ok(get_genesis_chain(Some(bill3.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification().returning(|_| None);
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);
    let res_all_comp = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::All,
            &[],
            &BillParticipant::Ident(bill_identified_participant_only_node_id(
                company_node_id.clone(),
            )),
            &company_keys,
        )
        .await;
    assert!(res_all_comp.is_ok());
    assert_eq!(res_all_comp.as_ref().unwrap().len(), 2);
    let res_all = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::All,
            &[],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_all.is_ok());
    assert_eq!(res_all.as_ref().unwrap().len(), 3);

    let res_term = service
        .search_bills(
            &Currency::sat(),
            &Some(String::from("hayek")),
            None,
            None,
            &BillsFilterRole::All,
            &[],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_term.is_ok());
    assert_eq!(res_term.as_ref().unwrap().len(), 1);

    let from_ts = Date::new("2030-05-01").unwrap().to_timestamp();
    let to_ts = Date::new("2030-05-30").unwrap().to_timestamp();
    let res_fromto = service
        .search_bills(
            &Currency::sat(),
            &None,
            Some(from_ts),
            Some(to_ts),
            &BillsFilterRole::All,
            &[],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_fromto.is_ok());
    assert_eq!(res_fromto.as_ref().unwrap().len(), 2);

    let res_role = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::Payer,
            &[],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_role.is_ok());
    assert_eq!(res_role.as_ref().unwrap().len(), 1);

    let res_part = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::All,
            std::slice::from_ref(&company_node_id),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_part.is_ok());
    assert_eq!(res_part.as_ref().unwrap().len(), 2); // company is drawee in bill 2, payee in bill 3

    let res_parts = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::All,
            &[company_node_id.clone(), bill1_payee.node_id()],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_parts.is_ok());
    assert_eq!(res_parts.as_ref().unwrap().len(), 0); // bill 1 payer is only in bill 1, company is not in bill 1

    let res_parts2 = service
        .search_bills(
            &Currency::sat(),
            &None,
            None,
            None,
            &BillsFilterRole::All,
            &[company_node_id, payee.node_id],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_parts2.is_ok());
    assert_eq!(res_parts2.as_ref().unwrap().len(), 2); // company and identity are in bill 2 and bill 3

    let res_comb = service
        .search_bills(
            &Currency::sat(),
            &Some(String::from("hayek")),
            Some(from_ts),
            Some(to_ts),
            &BillsFilterRole::Payee,
            &[],
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res_comb.is_ok());
    assert_eq!(res_comb.as_ref().unwrap().len(), 1);
}

#[tokio::test]
async fn issue_bill_baseline() {
    let mut ctx = get_ctx();
    let expected_file_name = Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap();
    let file_bytes = String::from("hello world").as_bytes().to_vec();

    ctx.file_upload_store
        .expect_read_temp_upload_file()
        .returning(move |_| {
            Ok((
                Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap(),
                file_bytes.clone(),
            ))
        });
    ctx.file_upload_store
        .expect_remove_temp_upload_folder()
        .returning(|_| Ok(()));
    ctx.file_upload_client.expect_upload().returning(|_, _| {
        Ok(bitcoin::hashes::sha256::Hash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap())
    });
    ctx.bill_store.expect_save_keys().returning(|_, _| Ok(()));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    // should send a bill is signed event
    ctx.transport_service
        .expect_send_bill_is_signed_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let drawer = get_baseline_identity();
    let mut drawee = empty_bill_identified_participant();
    drawee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    let mut payee = empty_bill_identified_participant();
    payee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    let bill = service
        .issue_new_bill(BillIssueData {
            t: 2,
            country_of_issuing: Country::GB,
            city_of_issuing: City::new("London").unwrap(),
            issue_date: Date::new("2030-01-01").unwrap(),
            maturity_date: Date::new("2030-04-01").unwrap(),
            drawee: drawee.node_id,
            payee: payee.node_id,
            sum: Sum::new_sat(100).expect("sat works"),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            file_upload_ids: vec![get_uuid_v4()],
            drawer_public_data: BillParticipant::Ident(
                BillIdentParticipant::new(drawer.identity).unwrap(),
            ),
            drawer_keys: drawer.key_pair,
            timestamp: test_ts(),
            blank_issue: false,
        })
        .await
        .unwrap();

    assert_eq!(bill.files.first().unwrap().name, expected_file_name);
    assert!(matches!(bill.payee, BillParticipant::Ident(_))); // payee is ident
}

#[tokio::test]
async fn issue_bill_baseline_anon() {
    let mut ctx = get_ctx();
    let expected_file_name = Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap();
    let file_bytes = String::from("hello world").as_bytes().to_vec();
    let mut payee = BillAnonParticipant::from(empty_bill_identified_participant());
    payee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    ctx.file_upload_store
        .expect_read_temp_upload_file()
        .returning(move |_| {
            Ok((
                Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap(),
                file_bytes.clone(),
            ))
        });
    ctx.file_upload_store
        .expect_remove_temp_upload_folder()
        .returning(|_| Ok(()));
    ctx.file_upload_client.expect_upload().returning(|_, _| {
        Ok(bitcoin::hashes::sha256::Hash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap())
    });
    ctx.bill_store.expect_save_keys().returning(|_, _| Ok(()));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    // should send a bill is signed event
    ctx.transport_service
        .expect_send_bill_is_signed_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let drawer = get_baseline_identity();
    let mut drawee = empty_bill_identified_participant();
    drawee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    let bill = service
        .issue_new_bill(BillIssueData {
            t: 2,
            country_of_issuing: Country::GB,
            city_of_issuing: City::new("London").unwrap(),
            issue_date: Date::new("2030-01-01").unwrap(),
            maturity_date: Date::new("2030-04-01").unwrap(),
            drawee: drawee.node_id,
            payee: payee.node_id,
            sum: Sum::new_sat(100).expect("sat works"),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            file_upload_ids: vec![get_uuid_v4()],
            drawer_public_data: BillParticipant::Ident(
                BillIdentParticipant::new(drawer.identity).unwrap(),
            ),
            drawer_keys: drawer.key_pair,
            timestamp: test_ts(),
            blank_issue: true,
        })
        .await
        .unwrap();

    assert_eq!(bill.files.first().unwrap().name, expected_file_name);
    assert!(matches!(bill.payee, BillParticipant::Anon(_))); // payee is anon
}

#[tokio::test]
async fn issue_bill_fails_for_anon_drawer() {
    let ctx = get_ctx();
    let service = get_service(ctx);

    let drawer = get_baseline_identity();
    let mut drawee = empty_bill_identified_participant();
    drawee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    let mut payee = empty_bill_identified_participant();
    payee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    let result = service
        .issue_new_bill(BillIssueData {
            t: 2,
            country_of_issuing: Country::GB,
            city_of_issuing: City::new("London").unwrap(),
            issue_date: Date::new("2030-01-01").unwrap(),
            maturity_date: Date::new("2030-04-01").unwrap(),
            drawee: drawee.node_id,
            payee: payee.node_id,
            sum: Sum::new_sat(100).expect("sat works"),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            file_upload_ids: vec![get_uuid_v4()],
            drawer_public_data: BillParticipant::Anon(BillAnonParticipant::new(drawer.identity)),
            drawer_keys: drawer.key_pair,
            timestamp: test_ts(),
            blank_issue: false,
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SignerCantBeAnon
        ))
    ));
}

#[tokio::test]
async fn issue_bill_fails_for_self_drafted_blank() {
    let ctx = get_ctx();
    let service = get_service(ctx);

    let drawer = get_baseline_identity();
    let mut drawee = empty_bill_identified_participant();
    drawee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    let mut payee = empty_bill_identified_participant();
    payee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    let result = service
        .issue_new_bill(BillIssueData {
            t: 1,
            country_of_issuing: Country::GB,
            city_of_issuing: City::new("London").unwrap(),
            issue_date: Date::new("2030-01-01").unwrap(),
            maturity_date: Date::new("2030-04-01").unwrap(),
            drawee: drawee.node_id,
            payee: payee.node_id,
            sum: Sum::new_sat(100).expect("sat works"),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            file_upload_ids: vec![get_uuid_v4()],
            drawer_public_data: BillParticipant::Ident(
                BillIdentParticipant::new(drawer.identity).unwrap(),
            ),
            drawer_keys: drawer.key_pair,
            timestamp: test_ts(),
            blank_issue: true,
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SelfDraftedBillCantBeBlank
        ))
    ));
}

#[tokio::test]
async fn issue_bill_as_company() {
    let mut ctx = get_ctx();
    let expected_file_name = Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap();
    let file_bytes = String::from("hello world").as_bytes().to_vec();

    ctx.file_upload_store
        .expect_read_temp_upload_file()
        .returning(move |_| {
            Ok((
                Name::new("invoice_00000000-0000-0000-0000-000000000000.pdf").unwrap(),
                file_bytes.clone(),
            ))
        });
    ctx.file_upload_store
        .expect_remove_temp_upload_folder()
        .returning(|_| Ok(()));
    ctx.file_upload_client.expect_upload().returning(|_, _| {
        Ok(bitcoin::hashes::sha256::Hash::from_str(
            "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
        )
        .unwrap())
    });
    ctx.bill_store.expect_save_keys().returning(|_, _| Ok(()));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    // should send a bill is signed event
    ctx.transport_service
        .expect_send_bill_is_signed_event()
        .returning(|_| Ok(()));
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_other_identity_proof_test()]));

    // Populates company and identity block
    expect_populates_company_and_identity_block(&mut ctx);

    let service = get_service(ctx);

    let drawer = get_baseline_company_data();
    let mut drawee = empty_bill_identified_participant();
    drawee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    let mut payee = empty_bill_identified_participant();
    payee.node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);

    let bill = service
        .issue_new_bill(BillIssueData {
            t: 2,
            country_of_issuing: Country::GB,
            city_of_issuing: City::new("London").unwrap(),
            issue_date: Date::new("2030-01-01").unwrap(),
            maturity_date: Date::new("2030-04-01").unwrap(),
            drawee: drawee.node_id,
            payee: payee.node_id,
            sum: Sum::new_sat(100).expect("sat works"),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            file_upload_ids: vec![get_uuid_v4()],
            drawer_public_data: BillParticipant::Ident(BillIdentParticipant::from(drawer.1.0)),
            drawer_keys: BcrKeys::from_private_key(&drawer.1.1.get_private_key()),
            timestamp: test_ts(),
            blank_issue: false,
        })
        .await
        .unwrap();

    assert_eq!(bill.files.first().unwrap().name, expected_file_name);
    assert_eq!(bill.drawer.node_id, drawer.0);
}

#[tokio::test]
async fn open_decrypt_propagates_download_error() {
    let mut ctx = get_ctx();
    ctx.file_upload_client.expect_download().returning(|_, _| {
        Err(crate::external::Error::ExternalFileStorageApi(
            crate::external::file_storage::Error::InvalidRelayUrl,
        ))
    });
    let service = get_service(ctx);

    assert!(
        service
            .open_and_decrypt_attached_file(
                &bill_id_test(),
                &File {
                    name: Name::new("some_file").unwrap(),
                    hash: Sha256Hash::new("some hash"),
                    nostr_hash: Sha256HexHash::from_str(
                        "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                    )
                    .unwrap(),
                },
                &private_key_test(),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn open_decrypt_uses_bill_private_key_for_new_attachments() {
    let mut ctx = get_ctx();
    let bill_keys = BcrKeys::from_private_key(&private_key_test_another());
    let plaintext = b"bill-key-encrypted-file".to_vec();
    let encrypted =
        bcr_ebill_core::protocol::crypto::encrypt_ecies(&plaintext, &bill_keys.pub_key()).unwrap();

    let encrypted_clone = encrypted.clone();
    ctx.file_upload_client
        .expect_download()
        .returning(move |_, _| Ok(encrypted_clone.clone()));
    let service = get_service(ctx);

    let result = service
        .open_and_decrypt_attached_file(
            &bill_id_test(),
            &File {
                name: Name::new("some_file").unwrap(),
                hash: Sha256Hash::from_bytes(&plaintext),
                nostr_hash: Sha256HexHash::from_str(
                    "d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa",
                )
                .unwrap(),
            },
            &private_key_test_another(),
        )
        .await
        .unwrap();

    assert_eq!(result, plaintext);
}

#[tokio::test]
async fn upload_bill_files_for_node_id_reencrypts_for_receiver_key() {
    let mut ctx = get_ctx();
    let bill_keys = BcrKeys::from_private_key(&private_key_test());
    let receiver_keys = BcrKeys::new();
    let plaintext = b"shared-with-external-party".to_vec();
    let stored_encrypted =
        bcr_ebill_core::protocol::crypto::encrypt_ecies(&plaintext, &bill_keys.pub_key()).unwrap();
    let expected_hash =
        Sha256HexHash::from_str("d277fe40da2609ca08215cdfbeac44835d4371a72f1416a63c87efd67ee24bfa")
            .unwrap();

    ctx.identity_store
        .expect_get_current_identity()
        .returning(|| {
            Ok(bcr_ebill_core::application::identity::ActiveIdentityState {
                personal: node_id_test(),
                company: None,
            })
        });
    let stored_clone = stored_encrypted.clone();
    ctx.file_upload_client
        .expect_download()
        .returning(move |_, _| Ok(stored_clone.clone()))
        .once();
    let plaintext_clone = plaintext.clone();
    let receiver_keys_for_upload = receiver_keys.clone();
    let receiver_keys_for_signing = receiver_keys_for_upload.clone();
    ctx.file_upload_client
        .expect_upload()
        .returning(move |_, bytes| {
            let decrypted = bcr_ebill_core::protocol::crypto::decrypt_ecies(
                &bytes,
                &receiver_keys_for_upload.get_private_key(),
            )
            .unwrap();
            assert_eq!(decrypted, plaintext_clone);
            Ok(expected_hash)
        })
        .once();

    let service = get_service(ctx);
    let urls = service
        .upload_bill_files_for_node_id(
            &bill_id_test(),
            &private_key_test(),
            &receiver_keys.pub_key(),
            &receiver_keys_for_signing,
            &[File {
                name: Name::new("some_file").unwrap(),
                hash: Sha256Hash::from_bytes(&plaintext),
                nostr_hash: Sha256HexHash::from_str(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .unwrap(),
            }],
        )
        .await
        .unwrap();

    assert_eq!(urls.len(), 1);
    assert!(
        urls[0]
            .as_str()
            .ends_with(expected_hash.to_string().as_str())
    );
}

#[tokio::test]
async fn get_bill_keys_calls_storage() {
    let mut ctx = get_ctx();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let service = get_service(ctx);

    assert!(service.get_bill_keys(&bill_id_test()).await.is_ok());
    assert_eq!(
        service
            .get_bill_keys(&bill_id_test())
            .await
            .unwrap()
            .get_private_key(),
        private_key_test(),
    );
    assert_eq!(
        service
            .get_bill_keys(&bill_id_test())
            .await
            .unwrap()
            .pub_key(),
        node_id_test().pub_key(),
    );
}

#[tokio::test]
async fn get_bill_keys_propagates_errors() {
    let mut ctx = get_ctx();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_get_keys().returning(|_| {
        Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
            "test error",
        )))
    });
    let service = get_service(ctx);
    assert!(service.get_bill_keys(&bill_id_test()).await.is_err());
}

#[tokio::test]
async fn get_bills_baseline() {
    let mut ctx = get_ctx();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let chain = get_genesis_chain(Some(bill.clone()));
            Ok(chain)
        });
    ctx.bill_store
        .expect_get_ids()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));

    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);
    let identity = get_baseline_identity();

    let res = service
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    let returned_bills = res.unwrap();
    assert!(returned_bills.len() == 1);
    assert_eq!(returned_bills[0].id, bill_id_test());
}

#[tokio::test]
async fn get_bills_baseline_from_cache() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut chain_bill = get_baseline_bill(&bill_id_test_other());
    chain_bill.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    let mut bill = get_baseline_cached_bill(bill_id_test());
    bill.data.maturity_date = Date::new("2099-07-01").unwrap();
    // make sure the local identity is part of the bill
    bill.participants.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    bill.participants
        .all_participant_node_ids
        .push(identity.identity.node_id.clone());

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let chain = get_genesis_chain(Some(chain_bill.clone()));
            Ok(chain)
        })
        .times(1);
    ctx.bill_store
        .expect_get_bills_from_cache()
        .returning(move |_, _| Ok(vec![bill.clone()]));
    ctx.bill_store
        .expect_get_ids()
        .returning(|| Ok(vec![bill_id_test(), bill_id_test_other()]));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));

    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);

    let res = service
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    let returned_bills = res.unwrap();
    assert!(returned_bills.len() == 2);
}

#[tokio::test]
async fn get_bills_baseline_from_cache_with_payment_expiration() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut chain_bill = get_baseline_bill(&bill_id_test_other());
    chain_bill.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    let mut bill = get_baseline_cached_bill(bill_id_test());
    // make sure the local identity is part of the bill
    bill.participants.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    bill.participants
        .all_participant_node_ids
        .push(identity.identity.node_id.clone());
    bill.status.payment = BillPaymentStatus {
        time_of_request_to_pay: Some(Timestamp::new(1531593928).unwrap()), // more than 2 days before request
        requested_to_pay: true,
        paid: false,
        request_to_pay_timed_out: false,
        rejected_to_pay: false,
        payment_deadline_timestamp: Some(
            Timestamp::new(1531593928).unwrap() + 2 * PAYMENT_DEADLINE_SECONDS,
        ),
    };

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let chain = get_genesis_chain(Some(chain_bill.clone()));
            Ok(chain)
        })
        .times(2);
    ctx.bill_store
        .expect_get_bills_from_cache()
        .returning(move |_, _| Ok(vec![bill.clone()]));
    ctx.bill_store
        .expect_get_ids()
        .returning(|| Ok(vec![bill_id_test(), bill_id_test_other()]));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));

    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);

    let res = service
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    let returned_bills = res.unwrap();
    assert!(returned_bills.len() == 2);
}

#[tokio::test]
async fn get_bills_baseline_company() {
    let mut ctx = get_ctx();
    let company_keys = BcrKeys::new();
    let company_node_id = NodeId::new(company_keys.pub_key(), bitcoin::Network::Testnet);
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(|_| Ok(get_genesis_chain(None)));
    ctx.bill_store
        .expect_get_ids()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));

    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let service = get_service(ctx);

    let identity = get_baseline_identity();
    let res = service
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    let returned_bills = res.unwrap();
    assert!(returned_bills.len() == 1);
    assert_eq!(returned_bills[0].id, bill_id_test());

    let res = service
        .get_bills(
            &BillParticipant::Ident(bill_identified_participant_only_node_id(
                company_node_id.clone(),
            )),
            &company_keys,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().len(), 0);
}

#[tokio::test]
async fn get_bills_req_to_pay() {
    let mut ctx = get_ctx();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now();
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = BillBlock::create_block_for_request_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToPayBlockData {
                    requester: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    payment_data: BillPaymentBlockData {
                        sum: bill.sum.clone(),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * PAYMENT_DEADLINE_SECONDS,
                    },
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store
        .expect_get_ids()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });

    let identity = get_baseline_identity();
    let res = get_service(ctx)
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    let returned_bills = res.unwrap();
    assert!(returned_bills.len() == 1);
    assert_eq!(returned_bills[0].id, bill_id_test());
    assert!(returned_bills[0].status.payment.requested_to_pay);
    assert!(returned_bills[0].status.payment.paid);
}

#[tokio::test]
async fn get_bills_empty_for_no_bills() {
    let mut ctx = get_ctx();
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notifications()
            .returning(|_| HashMap::new());
    });
    ctx.bill_store.expect_get_ids().returning(|| Ok(vec![]));
    let identity = get_baseline_identity();
    let res = get_service(ctx)
        .get_bills(
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
    assert!(res.unwrap().is_empty());
}

#[tokio::test]
async fn get_detail_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(!res.as_ref().unwrap().status.acceptance.accepted);
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::Accept)
    );
    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::RejectAcceptance)
    );
    assert_eq!(res.as_ref().unwrap().actions.bill_actions.len(), 2);
}

#[tokio::test]
async fn get_detail_bill_baseline_from_cache() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_cached_bill(bill_id_test());
    bill.data.maturity_date = Date::new("2099-07-01").unwrap();
    // make sure the local identity is part of the bill
    bill.participants.drawee = BillIdentParticipant::new(identity.identity.clone()).unwrap();
    let drawee_node_id = bill.participants.drawee.node_id.clone();
    bill.participants
        .all_participant_node_ids
        .push(drawee_node_id.clone());
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store
        .expect_get_bill_from_cache()
        .returning(move |_, _| Ok(Some(bill.clone())));
    ctx.bill_blockchain_store.expect_get_chain().never();
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
}

#[tokio::test]
async fn get_detail_bill_baseline_from_cache_with_payment_expiration() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut chain_bill = get_baseline_bill(&bill_id_test());
    chain_bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let mut bill = get_baseline_cached_bill(bill_id_test());
    // make sure the local identity is part of the bill
    bill.participants.drawee = BillIdentParticipant::new(identity.identity.clone()).unwrap();
    let drawee_node_id = bill.participants.drawee.node_id.clone();
    bill.participants
        .all_participant_node_ids
        .push(drawee_node_id.clone());
    bill.status.payment = BillPaymentStatus {
        time_of_request_to_pay: Some(Timestamp::new(1531593928).unwrap()), // more than 2 days before request
        requested_to_pay: true,
        paid: false,
        request_to_pay_timed_out: false,
        rejected_to_pay: false,
        payment_deadline_timestamp: Some(
            Timestamp::new(1531593928).unwrap() + 2 * PAYMENT_DEADLINE_SECONDS,
        ),
    };
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store
        .expect_get_bill_from_cache()
        .returning(move |_, _| Ok(Some(bill.clone())));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(chain_bill.clone()))))
        .times(1);
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
}

#[tokio::test]
async fn get_detail_bill_baseline_error_from_cache() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store
        .expect_get_bill_from_cache()
        .returning(move |_, _| {
            Err(bcr_ebill_persistence::Error::Io(std::io::Error::other(
                "test error",
            )))
        });
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
}

#[tokio::test]
async fn get_detail_bill_fails_for_non_participant() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });
    let keys = BcrKeys::new();

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &bill_participant_only_node_id(NodeId::new(keys.pub_key(), bitcoin::Network::Testnet)),
            &keys,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn get_detail_waiting_for_offer_to_sell() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone()),
                None,
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.sell.offered_to_sell);
    assert!(!res.as_ref().unwrap().status.sell.offer_to_sell_timed_out);
    assert!(!res.as_ref().unwrap().status.sell.rejected_offer_to_sell);
    assert!(res.as_ref().unwrap().current_waiting_state.is_some());
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
    assert!(res.as_ref().unwrap().status.has_requested_funds);

    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::RejectBuying)
    );
    assert_eq!(res.as_ref().unwrap().actions.bill_actions.len(), 1);
}

#[tokio::test]
async fn get_detail_waiting_for_offer_to_sell_and_sell() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill.drawee,
                None,
            )));
            assert!(chain.try_add_block(sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill.drawee,
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.sell.offered_to_sell);
    assert!(!res.as_ref().unwrap().status.sell.offer_to_sell_timed_out);
    assert!(!res.as_ref().unwrap().status.sell.rejected_offer_to_sell);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert_eq!(
        res.as_ref()
            .unwrap()
            .participants
            .endorsee
            .as_ref()
            .unwrap()
            .node_id(),
        identity.identity.node_id
    );
    assert!(res.as_ref().unwrap().status.redeemed_funds_available); // caller is endorsee
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_waiting_for_offer_to_sell_and_expire() {
    let mut ctx = get_ctx();
    let now = Timestamp::now();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill.drawee,
                // expired
                Some(now - PAYMENT_DEADLINE_SECONDS * 2),
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.sell.offered_to_sell);
    assert!(res.as_ref().unwrap().status.sell.offer_to_sell_timed_out);
    assert!(!res.as_ref().unwrap().status.sell.rejected_offer_to_sell);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
    assert_eq!(res.as_ref().unwrap().actions.bill_actions.len(), 2);
    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::Accept)
    );
    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::RejectAcceptance)
    );
}

#[tokio::test]
async fn get_detail_waiting_for_offer_to_sell_and_reject() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let now = Timestamp::now();
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill.drawee,
                // expired
                Some(now - PAYMENT_DEADLINE_SECONDS * 2),
            )));
            assert!(
                chain.try_add_block(reject_buy_block(&bill_id_test(), chain.get_latest_block(),))
            );
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.sell.offered_to_sell);
    assert!(!res.as_ref().unwrap().status.sell.offer_to_sell_timed_out);
    assert!(res.as_ref().unwrap().status.sell.rejected_offer_to_sell);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_recourse() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone()),
                None,
            );
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.recourse.requested_to_recourse);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .request_to_recourse_timed_out
    );
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .rejected_request_to_recourse
    );
    assert!(res.as_ref().unwrap().current_waiting_state.is_some());
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
    assert!(res.as_ref().unwrap().status.has_requested_funds);
    assert_eq!(res.as_ref().unwrap().actions.bill_actions.len(), 1);
    assert!(
        res.as_ref()
            .unwrap()
            .actions
            .bill_actions
            .contains(&BillCallerBillAction::RejectPaymentForRecourse)
    );
}

#[tokio::test]
async fn get_detail_bill_req_to_recourse_recoursed() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone()),
                None,
            );
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone())
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id.clone()
    );
    assert!(res.as_ref().unwrap().status.recourse.requested_to_recourse);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .request_to_recourse_timed_out
    );
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .rejected_request_to_recourse
    );
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.redeemed_funds_available); // caller is endorsee
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_recourse_rejected() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone()),
                None,
            );
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(reject_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.recourse.requested_to_recourse);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .request_to_recourse_timed_out
    );
    assert!(
        res.as_ref()
            .unwrap()
            .status
            .recourse
            .rejected_request_to_recourse
    );
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_recourse_expired() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let now = Timestamp::now();
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(bill.drawee.node_id.clone()),
                Some(now - RECOURSE_DEADLINE_SECONDS * 3),
            );
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.recourse.requested_to_recourse);
    assert!(
        res.as_ref()
            .unwrap()
            .status
            .recourse
            .request_to_recourse_timed_out
    );
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .recourse
            .rejected_request_to_recourse
    );
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_pay_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(!res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_some());
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available);
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay_paid() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_pay_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });
    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(!res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(!res.as_ref().unwrap().status.redeemed_funds_available); // caller not payee
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay_rejected() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_pay_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(reject_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block()
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay_rejected_but_paid() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_pay_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(reject_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block()
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay_expired() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let now = Timestamp::now();
    bill.maturity_date = Date::from(now - PAYMENT_DEADLINE_SECONDS * 2);
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block(),
                Some(now - PAYMENT_DEADLINE_SECONDS * 3),
            );
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(!res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_pay_expired_but_paid() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    let now = Timestamp::now();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block(),
                Some(now - PAYMENT_DEADLINE_SECONDS * 2),
            );
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.payment.paid);
    assert!(res.as_ref().unwrap().status.payment.requested_to_pay);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .payment
            .request_to_pay_timed_out
    );
    assert!(!res.as_ref().unwrap().status.payment.rejected_to_pay);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
    assert!(res.as_ref().unwrap().status.has_requested_funds);
}

#[tokio::test]
async fn get_detail_bill_req_to_accept() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_accept_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.acceptance.accepted);
    assert!(res.as_ref().unwrap().status.acceptance.requested_to_accept);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .acceptance
            .request_to_accept_timed_out
    );
    assert!(!res.as_ref().unwrap().status.acceptance.rejected_to_accept);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
}

#[tokio::test]
async fn get_detail_bill_req_to_accept_accepted() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_accept_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(accept_block(&bill_id_test(), chain.get_latest_block())));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(res.as_ref().unwrap().status.acceptance.accepted);
    assert!(res.as_ref().unwrap().status.acceptance.requested_to_accept);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .acceptance
            .request_to_accept_timed_out
    );
    assert!(!res.as_ref().unwrap().status.acceptance.rejected_to_accept);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
}

#[tokio::test]
async fn get_detail_bill_req_to_accept_rejected() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    let now = Timestamp::now();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block =
                request_to_accept_block(&bill_id_test(), chain.get_latest_block(), None);
            assert!(chain.try_add_block(req_to_pay_block));
            assert!(chain.try_add_block(reject_accept_block(
                &bill_id_test(),
                chain.get_latest_block()
            )));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.acceptance.accepted);
    assert!(res.as_ref().unwrap().status.acceptance.requested_to_accept);
    assert!(
        !res.as_ref()
            .unwrap()
            .status
            .acceptance
            .request_to_accept_timed_out
    );
    assert!(res.as_ref().unwrap().status.acceptance.rejected_to_accept);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
}

#[tokio::test]
async fn get_detail_bill_req_to_accept_expired() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let now = Timestamp::now();
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawee_node_id = bill.drawee.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_pay_block = request_to_accept_block(
                &bill_id_test(),
                chain.get_latest_block(),
                Some(now - ACCEPT_DEADLINE_SECONDS * 3),
            );
            assert!(chain.try_add_block(req_to_pay_block));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            now,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().id, bill_id_test());
    assert_eq!(
        res.as_ref().unwrap().participants.drawee.node_id,
        drawee_node_id
    );
    assert!(!res.as_ref().unwrap().status.acceptance.accepted);
    assert!(res.as_ref().unwrap().status.acceptance.requested_to_accept);
    assert!(
        res.as_ref()
            .unwrap()
            .status
            .acceptance
            .request_to_accept_timed_out
    );
    assert!(!res.as_ref().unwrap().status.acceptance.rejected_to_accept);
    assert!(res.as_ref().unwrap().current_waiting_state.is_none());
}

#[tokio::test]
async fn accept_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Should send bill accepted event
    ctx.transport_service
        .expect_send_bill_is_accepted_event()
        .returning(|_| Ok(()));

    // Populate identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Accept,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::Accept);
}

#[tokio::test]
async fn accept_bill_fails_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Accept,
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
    assert!(matches!(
        res.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SignerCantBeAnon
        ))
    ));
}

fn expect_populates_identity_block(ctx: &mut MockBillContext) {
    ctx.transport_service.expect_on_block_transport(|t| {
        t.expect_send_identity_chain_events()
            .returning(|_| Ok(()))
            .once();
    });
}

fn expect_populates_company_and_identity_block(ctx: &mut MockBillContext) {
    ctx.company_chain_store
        .expect_get_chain()
        .returning(|_| Ok(get_valid_company_chain()));
    ctx.company_store
        .expect_get()
        .returning(|_| Ok(get_baseline_company()));
    ctx.transport_service.expect_on_block_transport(|t| {
        t.expect_send_company_chain_events()
            .returning(|_| Ok(()))
            .once();
        t.expect_send_identity_chain_events()
            .returning(|_| Ok(()))
            .once();
    });
}

fn expect_populates_company_and_identity_block_with_sync(
    ctx: &mut MockBillContext,
    tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
) {
    ctx.company_chain_store
        .expect_get_chain()
        .returning(|_| Ok(get_valid_company_chain()));
    ctx.company_store
        .expect_get()
        .returning(|_| Ok(get_baseline_company()));
    ctx.transport_service.expect_on_block_transport(move |t| {
        t.expect_send_company_chain_events()
            .returning(|_| Ok(()))
            .once();
        let tx = Arc::clone(&tx);
        t.expect_send_identity_chain_events()
            .returning(move |_| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                Ok(())
            })
            .once();
    });
}

#[tokio::test]
async fn accept_bill_as_company() {
    let mut ctx = get_ctx();
    let company = get_baseline_company_data();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(company.0.clone());

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_other_identity_proof_test()]));

    // Should send bill accepted event
    ctx.transport_service
        .expect_send_bill_is_accepted_event()
        .returning(|_| Ok(()));

    // Populate company and identity block via transport
    expect_populates_company_and_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Accept,
            &BillParticipant::Ident(BillIdentParticipant::from(company.1.0)),
            &BcrKeys::from_private_key(&company.1.1.get_private_key()),
            test_ts(),
        )
        .await;

    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.as_ref().unwrap().blocks()[1].op_code == BillOpCode::Accept);
    // company is accepter
    assert!(
        res.as_ref().unwrap().blocks()[1]
            .get_nodes_from_block(&BcrKeys::from_private_key(&private_key_test()))
            .unwrap()[0]
            == company.0
    );
}

#[tokio::test]
async fn accept_bill_fails_if_drawee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Accept,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn accept_bill_fails_if_already_accepted() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let keys = identity.key_pair.clone();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let mut chain = get_genesis_chain(Some(bill.clone()));
    let latest_block = chain.get_latest_block().clone();
    chain.blocks_mut().push(
        BillBlock::new(
            bill_id_test(),
            BlockId::next_from_previous_block_id(&latest_block.id()),
            Sha256Hash::new("prevhash"),
            Vec::new(),
            BillOpCode::Accept,
            &keys,
            None,
            &BcrKeys::from_private_key(&private_key_test()),
            test_ts(),
            Sha256Hash::new("plain text hash"),
        )
        .unwrap(),
    );
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(chain.clone()));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Accept,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn request_pay_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.maturity_date = Date::new("2022-11-12").unwrap(); // maturity date has to be in the past
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to pay event should be sent
    ctx.transport_service
        .expect_send_request_to_pay_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestToPay(
                Currency::sat(),
                safe_deadline_ts(PAYMENT_DEADLINE_SECONDS),
                None,
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::RequestToPay);
}

#[tokio::test]
async fn request_pay_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.maturity_date = Date::new("2022-11-12").unwrap(); // maturity date has to be in the past
    bill.payee = BillParticipant::Anon(BillAnonParticipant::from(
        bill_identified_participant_only_node_id(identity.identity.node_id.clone()),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to pay event should be sent
    ctx.transport_service
        .expect_send_request_to_pay_event()
        .returning(|_| Ok(()));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestToPay(
                Currency::sat(),
                safe_deadline_ts(PAYMENT_DEADLINE_SECONDS),
                None,
            ),
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::RequestToPay);
}

#[tokio::test]
async fn request_pay_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestToPay(
                Currency::sat(),
                safe_deadline_ts(PAYMENT_DEADLINE_SECONDS),
                None,
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn request_acceptance_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to accept event should be sent
    ctx.transport_service
        .expect_send_request_to_accept_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestAcceptance(safe_deadline_ts(ACCEPT_DEADLINE_SECONDS)),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::RequestToAccept);
}

#[tokio::test]
async fn request_acceptance_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Anon(BillAnonParticipant::from(
        bill_identified_participant_only_node_id(identity.identity.node_id.clone()),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to accept event should be sent
    ctx.transport_service
        .expect_send_request_to_accept_event()
        .returning(|_| Ok(()));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestAcceptance(safe_deadline_ts(ACCEPT_DEADLINE_SECONDS)),
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::RequestToAccept);
}

#[tokio::test]
async fn request_acceptance_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestAcceptance(safe_deadline_ts(ACCEPT_DEADLINE_SECONDS)),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn mint_bitcredit_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            chain.try_add_block(accept_block(&bill.id, chain.get_latest_block()));
            Ok(chain)
        });
    // Asset request to mint event is sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Mint(
                BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                ))),
                Sum::new_sat(5000).expect("sat works"),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 3);
    assert!(res.unwrap().blocks()[2].op_code == BillOpCode::Mint);
}

#[tokio::test]
async fn mint_bitcredit_bill_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            chain.try_add_block(accept_block(&bill.id, chain.get_latest_block()));
            Ok(chain)
        });
    // Asset request to mint event is sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Mint(
                BillParticipant::Anon(BillAnonParticipant::from(
                    bill_identified_participant_only_node_id(NodeId::new(
                        BcrKeys::new().pub_key(),
                        bitcoin::Network::Testnet,
                    )),
                )),
                Sum::new_sat(5000).expect("sat works"),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 3);
    assert!(res.unwrap().blocks()[2].op_code == BillOpCode::Mint);
}

#[tokio::test]
async fn mint_bitcredit_bill_fails_if_not_accepted() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Asset request to mint event is sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Mint(
                BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                ))),
                Sum::new_sat(5000).expect("sat works"),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn mint_bitcredit_bill_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Mint(
                BillParticipant::Ident(empty_bill_identified_participant()),
                Sum::new_sat(5000).expect("sat works"),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn offer_to_sell_bitcredit_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to sell event should be sent
    ctx.transport_service
        .expect_send_offer_to_sell_event()
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::OfferToSell(
                BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                ))),
                Sum::new_sat(15000).expect("sat works"),
                safe_deadline_ts(DAY_IN_SECS),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::OfferToSell);
}

#[tokio::test]
async fn offer_to_sell_bitcredit_bill_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to sell event should be sent
    ctx.transport_service
        .expect_send_offer_to_sell_event()
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::OfferToSell(
                BillParticipant::Anon(BillAnonParticipant::from(
                    bill_identified_participant_only_node_id(NodeId::new(
                        BcrKeys::new().pub_key(),
                        bitcoin::Network::Testnet,
                    )),
                )),
                Sum::new_sat(15000).expect("sat works"),
                safe_deadline_ts(DAY_IN_SECS),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::OfferToSell);
}

#[tokio::test]
async fn offer_to_sell_bitcredit_bill_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::OfferToSell(
                BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                ))),
                Sum::new_sat(15000).expect("sat works"),
                safe_deadline_ts(DAY_IN_SECS),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn sell_bitcredit_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    let buyer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let buyer_clone = buyer.clone();
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let offer_to_sell = BillBlock::create_block_for_offer_to_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillOfferToSellBlockData {
                    seller: bill.payee.clone().into(),
                    buyer: BillParticipantBlockData::Ident(buyer_clone.clone().into()),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * DAY_IN_SECS,
                    },
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(offer_to_sell);
            Ok(chain)
        });
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_create_local_bill_notification()
            .returning(|_, _, _, _, _| Ok(()));
    });
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Sell(BillParticipant::Ident(buyer)),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 3);
    assert!(res.as_ref().unwrap().blocks()[1].op_code == BillOpCode::OfferToSell);
    assert!(res.as_ref().unwrap().blocks()[2].op_code == BillOpCode::Sell);
}

#[tokio::test]
async fn sell_bitcredit_bill_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    let buyer = BillAnonParticipant::from(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    let buyer_clone = buyer.clone();
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let offer_to_sell = BillBlock::create_block_for_offer_to_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillOfferToSellBlockData {
                    seller: bill.payee.clone().into(),
                    buyer: BillParticipantBlockData::Anon(buyer_clone.clone().into()),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * DAY_IN_SECS,
                    },
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(offer_to_sell);
            Ok(chain)
        });
    // Request to sell event should be sent
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_create_local_bill_notification()
            .returning(|_, _, _, _, _| Ok(()));
    });
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Sell(BillParticipant::Anon(buyer)),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 3);
    assert!(res.as_ref().unwrap().blocks()[1].op_code == BillOpCode::OfferToSell);
    assert!(res.as_ref().unwrap().blocks()[2].op_code == BillOpCode::Sell);
}

#[tokio::test]
async fn sell_bitcredit_bill_fails_if_sell_data_is_invalid() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    let buyer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let offer_to_sell = BillBlock::create_block_for_offer_to_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillOfferToSellBlockData {
                    seller: bill.payee.clone().into(),
                    buyer: bill.payee.clone().into(), // buyer is seller, which is invalid
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(10000).expect("sat works"), // different sum
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * DAY_IN_SECS,
                    },
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(offer_to_sell);
            Ok(chain)
        });
    // Sold event should be sent
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Sell(BillParticipant::Ident(buyer)),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn sell_bitcredit_bill_fails_if_not_offer_to_sell_waiting_for_payment() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Request to sell event should be sent
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Sell(BillParticipant::Ident(
                bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                )),
            )),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn sell_bitcredit_bill_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Sell(BillParticipant::Ident(
                bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                )),
            )),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn endorse_bitcredit_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Bill is endorsed event should be sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    // Populates identity block
    expect_populates_identity_block(&mut ctx);
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Endorse(BillParticipant::Ident(
                bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                )),
            )),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::Endorse);
}

#[tokio::test]
async fn endorse_bitcredit_bill_multiple_back_and_forth() {
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));

    let party2_keys = BcrKeys::new();
    let party2_node_id = NodeId::new(party2_keys.pub_key(), bitcoin::Network::Testnet);
    let party2_participant = bill_identified_participant_only_node_id(party2_node_id.clone());

    let mut current_chain = get_genesis_chain(Some(bill.clone()));
    let mut current_timestamp = test_ts();

    for i in 0..10 {
        let is_even = i % 2 == 0;

        let mut ctx = get_ctx();
        ctx.bill_store
            .expect_save_bill_to_cache()
            .returning(|_, _, _| Ok(()))
            .times(1);
        ctx.bill_store.expect_exists().returning(|_| Ok(true));

        let chain_for_ctx = current_chain.clone();
        ctx.bill_blockchain_store
            .expect_get_chain()
            .returning(move |_| Ok(chain_for_ctx.clone()));

        ctx.transport_service
            .expect_send_bill_is_endorsed_event()
            .returning(|_| Ok(()));
        ctx.identity_store
            .expect_get_email_confirmations()
            .returning(|| Ok(vec![signed_identity_proof_test()]));

        expect_populates_identity_block(&mut ctx);

        let service = get_service(ctx);

        // Determine endorser and endorsee based on iteration
        let (endorser_participant, endorser_keys, endorsee_participant) = if is_even {
            // Even iterations: Party1 endorses to Party2
            (
                BillParticipant::Ident(
                    BillIdentParticipant::new(identity.identity.clone()).unwrap(),
                ),
                &identity.key_pair,
                BillParticipant::Ident(party2_participant.clone()),
            )
        } else {
            // Odd iterations: Party2 endorses to Party1
            (
                BillParticipant::Ident(party2_participant.clone()),
                &party2_keys,
                BillParticipant::Ident(
                    BillIdentParticipant::new(identity.identity.clone()).unwrap(),
                ),
            )
        };

        // Execute the endorsement
        let result = service
            .execute_bill_action(
                &bill_id_test(),
                BillAction::Endorse(endorsee_participant),
                &endorser_participant,
                endorser_keys,
                current_timestamp,
            )
            .await;

        assert!(result.is_ok(), "Endorsement {} failed", i + 1);
        current_chain = result.unwrap();

        // Verify the chain grows by one block each time
        // The chain should have 1 genesis block plus (i+1) endorsement blocks after each iteration.
        assert_eq!(current_chain.blocks().len(), 1 + (i + 1)); // Genesis + (i+1) endorsements
        assert_eq!(
            current_chain.blocks().last().unwrap().op_code,
            BillOpCode::Endorse
        );

        // Verify timestamp ordering: the new block's timestamp should be >= previous block's timestamp
        let blocks = current_chain.blocks();
        let new_block_index = blocks.len() - 1;
        if new_block_index > 0 {
            let prev_timestamp = blocks[new_block_index - 1].timestamp;
            let curr_timestamp = blocks[new_block_index].timestamp;
            assert!(
                curr_timestamp >= prev_timestamp,
                "Block {} timestamp ({}) is before previous block {} timestamp ({})",
                new_block_index,
                curr_timestamp,
                new_block_index - 1,
                prev_timestamp
            );
        }

        current_timestamp = current_timestamp + 1;
    }

    // Create a final context to verify the end state
    let mut final_ctx = get_ctx();
    final_ctx.bill_store.expect_exists().returning(|_| Ok(true));

    let final_chain = current_chain.clone();
    final_ctx
        .bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(final_chain.clone()));

    final_ctx
        .transport_service
        .expect_on_notification_transport(|t| {
            t.expect_get_active_bill_notification().returning(|_| None);
        });

    let final_service = get_service(final_ctx);

    // After back-and-forth endorsements, the bill should be back with Party1
    // (start with Party1 and do an even number of transfers)
    let bill_detail = final_service
        .get_detail(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            current_timestamp,
        )
        .await;
    assert!(bill_detail.is_ok());

    let bill_result = bill_detail.unwrap();
    assert_eq!(
        bill_result
            .participants
            .endorsee
            .as_ref()
            .unwrap()
            .node_id(),
        identity.identity.node_id,
    );

    // Verify the endorsement chain
    let endorsements = final_service
        .get_endorsements(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(endorsements.is_ok());

    // Should have 10 endorsements total
    assert_eq!(endorsements.as_ref().unwrap().len(), 10);

    // The most recent endorsement should be back to party1 (identity)
    assert_eq!(
        endorsements.as_ref().unwrap()[0]
            .pay_to_the_order_of
            .node_id(),
        identity.identity.node_id
    );
}

#[tokio::test]
async fn endorse_bitcredit_bill_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    // Bill is endorsed event should be sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Endorse(BillParticipant::Anon(BillAnonParticipant::from(
                bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                )),
            ))),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 2);
    assert!(res.unwrap().blocks()[1].op_code == BillOpCode::Endorse);
}

#[tokio::test]
async fn endorse_bitcredit_bill_anon_company_and_back() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let company_node_id = node_id_test_another();
    let company_participant =
        BillAnonParticipant::from(bill_participant_only_node_id(company_node_id.clone()));
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    let company_participant_clone = company_participant.clone();
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add endorse block from payee to endorsee
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Anon(
                        company_participant_clone.clone().into(),
                    ),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    signatory: None,
                    signing_timestamp: test_ts() - 14,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test_another())),
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 14,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block));
            Ok(chain)
        });
    // Bill is endorsed event should be sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_identity_proof_test()]))
        .times(1);
    ctx.company_store.expect_get().returning(|_| {
        let mut company = get_baseline_company_data();
        company.1.0.id = node_id_test_another();
        Ok(company.1.0)
    });

    // Populates identity block
    expect_populates_company_and_identity_block(&mut ctx);

    let service = get_service(ctx);

    let mut company_ident_participant = bill_participant_only_node_id(company_node_id.clone());
    if let BillParticipant::Ident(ref mut bill_ident_participant) = company_ident_participant {
        bill_ident_participant.t = ContactType::Company;
    };

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Endorse(BillParticipant::Ident(
                BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
            )),
            &company_ident_participant,
            &BcrKeys::from_private_key(&private_key_test_another()),
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 3);
    assert!(res.as_ref().unwrap().blocks()[1].op_code == BillOpCode::Endorse);
    assert!(res.as_ref().unwrap().blocks()[2].op_code == BillOpCode::Endorse);
    let last_endorse_data: BillEndorseBlockData = res
        .unwrap()
        .blocks()
        .get(2)
        .unwrap()
        .get_decrypted_block(&BcrKeys::from_private_key(&private_key_test()))
        .unwrap();
    assert!(last_endorse_data.signatory.as_ref().unwrap().name.is_none()); // name is not set, because company was anon signer
    assert!(last_endorse_data.signing_address.is_none()); // address is not set, because company was anon signer
}

#[tokio::test]
async fn endorse_bitcredit_bill_fails_if_waiting_for_offer_to_sell() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet
                )),
                None,
            )));
            Ok(chain)
        });

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Endorse(BillParticipant::Ident(
                bill_identified_participant_only_node_id(NodeId::new(
                    BcrKeys::new().pub_key(),
                    bitcoin::Network::Testnet,
                )),
            )),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
    match res {
        Ok(_) => panic!("expected an error"),
        Err(e) => match e {
            Error::Validation(ValidationError::Protocol(
                ProtocolValidationError::BillIsOfferedToSellAndWaitingForPayment,
            )) => (),
            _ => panic!("expected a different error"),
        },
    };
}

#[tokio::test]
async fn endorse_bitcredit_bill_fails_if_payee_not_caller() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Endorse(BillParticipant::Ident(empty_bill_identified_participant())),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn get_combined_bitcoin_keys_for_bill_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .get_combined_bitcoin_keys_for_bill(
            &bill_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
        )
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn get_combined_bitcoin_keys_for_bill_err() {
    let mut ctx = get_ctx();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    )));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let non_participant_keys = BcrKeys::new();
    let res = service
        .get_combined_bitcoin_keys_for_bill(
            &bill_id_test(),
            &BillParticipant::Ident(bill_identified_participant_only_node_id(NodeId::new(
                non_participant_keys.pub_key(),
                bitcoin::Network::Testnet,
            ))),
            &non_participant_keys,
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn check_bills_payment_baseline() {
    let mut ctx = get_ctx();
    let bill = get_baseline_bill(&bill_id_test());
    ctx.bill_store
        .expect_get_bill_ids_waiting_for_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_payment_state()
        .returning(|_, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service.check_bills_payment().await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_payment_for_bill_baseline() {
    let mut ctx = get_ctx();
    let bill = get_baseline_bill(&bill_id_test());
    ctx.bill_store
        .expect_get_bill_ids_waiting_for_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_payment_state()
        .returning(|_, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .check_payment_for_bill(&bill_id_test(), &get_baseline_identity().identity)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_bill_offer_to_sell_payment_baseline() {
    let mut ctx = get_ctx();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.bill_store
        .expect_get_bill_ids_waiting_for_sell_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_offer_to_sell_payment_state()
        .returning(|_, _, _| Ok(()));
    let buyer_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(buyer_node_id.clone()),
                Some(Timestamp::now()),
            )));
            Ok(chain)
        });
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_create_local_bill_notification()
            .returning(|_, _, _, _, _| Ok(()));
    });
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block and company block
    ctx.transport_service.expect_on_block_transport(|t| {
        t.expect_send_identity_chain_events().returning(|_| Ok(()));
    });

    let service = get_service(ctx);

    let res = service
        .check_offer_to_sell_payment_for_bill(&bill_id_test(), &get_baseline_identity())
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_bills_offer_to_sell_payment_company_is_seller() {
    let mut ctx = get_ctx();
    let mut identity = get_baseline_identity();
    identity.key_pair = BcrKeys::new();
    identity.identity.node_id = NodeId::new(identity.key_pair.pub_key(), bitcoin::Network::Testnet);

    let company = get_baseline_company_data();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(BillIdentParticipant::from(company.1.0.clone()));
    ctx.company_store.expect_get().returning(|_| {
        let mut company = get_baseline_company_data();
        company.1.0.id = node_id_test_another();
        Ok(company.1.0)
    });
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_identity_proof_test()]));

    ctx.bill_store
        .expect_get_bill_ids_waiting_for_sell_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_offer_to_sell_payment_state()
        .returning(|_, _, _| Ok(()));
    let company_clone = company.clone();
    ctx.company_store.expect_get_all().returning(move || {
        let mut map = HashMap::new();
        map.insert(
            company_clone.0.clone(),
            (company_clone.1.0.clone(), company_clone.1.1.clone()),
        );
        Ok(map)
    });
    let buyer_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &bill_identified_participant_only_node_id(buyer_node_id.clone()),
                Some(Timestamp::now() - 5),
            )));
            Ok(chain)
        });
    ctx.transport_service
        .expect_send_bill_is_sold_event()
        .returning(|_, _| Ok(()));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_create_local_bill_notification()
            .returning(|_, _, _, _, _| Ok(()));
    });
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));

    // Populates identity block and company block
    expect_populates_company_and_identity_block_with_sync(&mut ctx, tx); // fix test flakyness, waiting for the expectation

    let service = get_service(ctx);

    let res = service.check_bills_offer_to_sell_payment().await;
    assert!(res.is_ok());

    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("send_identity_chain_events was not called")
        .expect("notification channel dropped");
}

#[tokio::test]
async fn check_bills_timeouts_does_nothing_if_not_timed_out() {
    let mut ctx = get_ctx();
    let op_codes = HashSet::from([
        BillOpCode::RequestToAccept,
        BillOpCode::RequestToPay,
        BillOpCode::OfferToSell,
        BillOpCode::RequestRecourse,
    ]);

    // fetches bill ids
    ctx.bill_store
        .expect_get_bill_ids_with_op_codes_since()
        .with(eq(op_codes.clone()), eq(Timestamp::zero()))
        .returning(|_, _| Ok(vec![bill_id_test(), bill_id_test_other()]));
    // fetches bill chain accept
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_accept_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });
    // fetches bill chain pay
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test_other()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_pay_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });
    let service = get_service(ctx);

    // now is the same as block created time so no timeout should have happened
    let res = service
        .check_bills_timeouts(Timestamp::new(1000).unwrap())
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_bills_timeouts_does_nothing_if_notifications_are_already_sent() {
    let mut ctx = get_ctx();
    let op_codes = HashSet::from([
        BillOpCode::RequestToAccept,
        BillOpCode::RequestToPay,
        BillOpCode::OfferToSell,
        BillOpCode::RequestRecourse,
    ]);

    // fetches bill ids
    ctx.bill_store
        .expect_get_bill_ids_with_op_codes_since()
        .with(eq(op_codes.clone()), eq(Timestamp::zero()))
        .returning(|_, _| Ok(vec![bill_id_test(), bill_id_test_other()]));

    // fetches bill chain accept
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_accept_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });

    // fetches bill chain pay
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test_other()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_pay_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });
    ctx.transport_service.expect_on_notification_transport(|t| {
        // notification already sent
        t.expect_check_bill_notification_sent()
            .with(eq(bill_id_test()), eq(2), eq(ActionType::AcceptBill))
            .returning(|_, _, _| Ok(true));

        // notification already sent
        t.expect_check_bill_notification_sent()
            .with(eq(bill_id_test_other()), eq(2), eq(ActionType::PayBill))
            .returning(|_, _, _| Ok(true));
    });

    let service = get_service(ctx);

    let res = service
        .check_bills_timeouts(Timestamp::new(1100).unwrap() + PAYMENT_DEADLINE_SECONDS)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_bills_timeouts() {
    let mut ctx = get_ctx();
    let op_codes = HashSet::from([
        BillOpCode::RequestToAccept,
        BillOpCode::RequestToPay,
        BillOpCode::OfferToSell,
        BillOpCode::RequestRecourse,
    ]);

    // fetches bill ids
    ctx.bill_store
        .expect_get_bill_ids_with_op_codes_since()
        .with(eq(op_codes.clone()), eq(Timestamp::zero()))
        .returning(|_, _| Ok(vec![bill_id_test(), bill_id_test_other()]));

    // fetches bill chain accept
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_accept_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });

    // fetches bill chain pay
    ctx.bill_blockchain_store
        .expect_get_chain()
        .with(eq(bill_id_test_other()))
        .returning(|id| {
            let mut chain = get_genesis_chain(Some(get_baseline_bill(id)));
            chain.try_add_block(request_to_pay_block(id, chain.get_latest_block(), None));
            Ok(chain)
        });

    ctx.transport_service.expect_on_notification_transport(|t| {
        let recipient_check = function(|r: &Vec<BillParticipant>| r.len() >= 2);

        // notification not sent
        t.expect_check_bill_notification_sent()
            .with(eq(bill_id_test()), eq(2), eq(ActionType::AcceptBill))
            .returning(|_, _, _| Ok(false));

        // notification not sent
        t.expect_check_bill_notification_sent()
            .with(eq(bill_id_test_other()), eq(2), eq(ActionType::PayBill))
            .returning(|_, _, _| Ok(false));

        // send accept timeout notification
        t.expect_send_request_to_action_timed_out_event()
            .with(
                always(),
                eq(bill_id_test()),
                always(),
                eq(ActionType::AcceptBill),
                recipient_check.clone(),
                always(),
                always(),
                always(),
            )
            .returning(|_, _, _, _, _, _, _, _| Ok(()));

        // send pay timeout notification
        t.expect_send_request_to_action_timed_out_event()
            .with(
                always(),
                eq(bill_id_test_other()),
                always(),
                eq(ActionType::PayBill),
                recipient_check,
                always(),
                always(),
                always(),
            )
            .returning(|_, _, _, _, _, _, _, _| Ok(()));
        // marks accept bill timeout as sent
        t.expect_mark_bill_notification_sent()
            .with(eq(bill_id_test()), eq(2), eq(ActionType::AcceptBill))
            .returning(|_, _, _| Ok(()));

        // marks pay bill timeout as sent
        t.expect_mark_bill_notification_sent()
            .with(eq(bill_id_test_other()), eq(2), eq(ActionType::PayBill))
            .returning(|_, _, _| Ok(()));
    });

    // we should have at least two participants

    let service = get_service(ctx);

    let res = service
        .check_bills_timeouts(Timestamp::new(1100).unwrap() + PAYMENT_DEADLINE_SECONDS)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn get_endorsements_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawer = BillIdentParticipant::new(identity.identity.clone()).unwrap();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification().returning(|_| None);
    });

    let service = get_service(ctx);

    let res = service
        .get_endorsements(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().len(), 0);
}

#[tokio::test]
async fn get_endorsements_multi_with_anon() {
    let mut ctx = get_ctx();
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification().returning(|_| None);
    });
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let drawer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let mint_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    // endorsee is anon
    let endorse_endorsee = BillAnonParticipant::from(bill_identified_participant_only_node_id(
        NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet),
    ));
    let sell_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.drawer = drawer.clone();
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let mint_endorsee_clone = mint_endorsee.clone();
    let sell_endorsee_clone = sell_endorsee.clone();

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now();
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add endorse block from payee to endorsee
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block));

            // add offer to sell block from endorsee to sell endorsee
            let offer_to_sell_block = BillBlock::create_block_for_offer_to_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillOfferToSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 1000,
                    },
                    signatory: None,
                    signing_timestamp: now + 2,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 2,
            )
            .unwrap();
            assert!(chain.try_add_block(offer_to_sell_block));

            // add sell block from endorsee to sell endorsee
            let sell_block = BillBlock::create_block_for_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 3,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 3,
            )
            .unwrap();
            assert!(chain.try_add_block(sell_block));

            // add mint block from sell endorsee to mint endorsee
            let mint_block = BillBlock::create_block_for_mint(
                bill_id_test(),
                chain.get_latest_block(),
                &BillMintBlockData {
                    endorsee: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    // endorsed by sell endorsee
                    endorser: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    sum: Sum::new_sat(15000).expect("sat works"),
                    signatory: None,
                    signing_timestamp: now + 4,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 4,
            )
            .unwrap();
            assert!(chain.try_add_block(mint_block));

            Ok(chain)
        });

    let service = get_service(ctx);

    let res = service
        .get_endorsements(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    // with duplicates, anon are also counted
    assert_eq!(res.as_ref().unwrap().len(), 3);
    // mint was last, so it's first
    assert_eq!(
        res.as_ref().unwrap()[0].pay_to_the_order_of.node_id(),
        mint_endorsee_clone.node_id
    );
    assert_eq!(
        res.as_ref().unwrap()[1].pay_to_the_order_of.node_id(),
        sell_endorsee_clone.node_id
    );
    // endorsee is not in the list, since they're anon
}

#[tokio::test]
async fn get_endorsements_multi() {
    let mut ctx = get_ctx();
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification().returning(|_| None);
    });
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let drawer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let mint_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let endorse_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let sell_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.drawer = drawer.clone();
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let endorse_endorsee_clone = endorse_endorsee.clone();
    let mint_endorsee_clone = mint_endorsee.clone();
    let sell_endorsee_clone = sell_endorsee.clone();

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now();
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add endorse block from payee to endorsee
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block));

            // add offer to sell block from endorsee to sell endorsee
            let offer_to_sell_block = BillBlock::create_block_for_offer_to_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillOfferToSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 1000,
                    },
                    signatory: None,
                    signing_timestamp: now + 2,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 2,
            )
            .unwrap();
            assert!(chain.try_add_block(offer_to_sell_block));
            // add sell block from endorsee to sell endorsee
            let sell_block = BillBlock::create_block_for_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 3,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 3,
            )
            .unwrap();
            assert!(chain.try_add_block(sell_block));

            // add mint block from sell endorsee to mint endorsee
            let mint_block = BillBlock::create_block_for_mint(
                bill_id_test(),
                chain.get_latest_block(),
                &BillMintBlockData {
                    endorsee: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    // endorsed by sell endorsee
                    endorser: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    sum: Sum::new_sat(15000).expect("sat works"),
                    signatory: None,
                    signing_timestamp: now + 4,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 4,
            )
            .unwrap();
            assert!(chain.try_add_block(mint_block));

            Ok(chain)
        });

    let service = get_service(ctx);

    let res = service
        .get_endorsements(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    // with duplicates
    assert_eq!(res.as_ref().unwrap().len(), 3);
    // mint was last, so it's first
    assert_eq!(
        res.as_ref().unwrap()[0].pay_to_the_order_of.node_id(),
        mint_endorsee_clone.node_id
    );
    assert_eq!(
        res.as_ref().unwrap()[1].pay_to_the_order_of.node_id(),
        sell_endorsee_clone.node_id
    );
    assert_eq!(
        res.as_ref().unwrap()[2].pay_to_the_order_of.node_id(),
        endorse_endorsee_clone.node_id
    );
}

#[tokio::test]
async fn get_past_endorsees_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawer = BillIdentParticipant::new(identity.identity.clone()).unwrap();

    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .get_past_endorsees(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
    // if we're the drawee and drawer, there's no holder before us
    assert_eq!(res.as_ref().unwrap().len(), 0);
}

#[tokio::test]
async fn get_past_endorsees_fails_if_not_my_bill() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawer = BillIdentParticipant::new(identity.identity.clone()).unwrap();

    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .get_past_endorsees(&bill_id_test(), &node_id_test_other())
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn get_past_endorsees_3_party() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let drawer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.drawer = drawer.clone();
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    let service = get_service(ctx);

    let res = service
        .get_past_endorsees(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
    // if it's a 3 party bill and we're the payee, the drawer is a previous holder
    assert_eq!(res.as_ref().unwrap().len(), 1);
    assert_eq!(
        res.as_ref().unwrap()[0].pay_to_the_order_of.node_id,
        drawer.node_id
    );
}

#[tokio::test]
async fn get_past_endorsees_multi_with_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let drawer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let mint_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    // endorsee is anon
    let endorse_endorsee = BillAnonParticipant::from(bill_identified_participant_only_node_id(
        NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet),
    ));
    let sell_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));

    bill.drawer = drawer.clone();
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let drawer_clone = bill.drawer.clone();
    let endorse_endorsee_clone = endorse_endorsee.clone();
    let mint_endorsee_clone = mint_endorsee.clone();
    let sell_endorsee_clone = sell_endorsee.clone();

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now();
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add endorse block from payee to endorsee
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block));

            // add sell block from endorsee to sell endorsee
            let sell_block = BillBlock::create_block_for_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 2,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 2,
            )
            .unwrap();
            assert!(chain.try_add_block(sell_block));

            // add mint block from sell endorsee to mint endorsee
            let mint_block = BillBlock::create_block_for_mint(
                bill_id_test(),
                chain.get_latest_block(),
                &BillMintBlockData {
                    endorsee: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    // endorsed by sell endorsee
                    endorser: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    sum: Sum::new_sat(15000).expect("sat works"),
                    signatory: None,
                    signing_timestamp: now + 3,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 3,
            )
            .unwrap();
            assert!(chain.try_add_block(mint_block));

            // add endorse block back to endorsee
            let endorse_block_back = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 4,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 4,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block_back));

            // add endorse block back to payee (caller)
            let endorse_block_last = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Anon(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 5,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 5,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block_last));

            Ok(chain)
        });
    let service = get_service(ctx);

    // Caller = Payee (=identity)
    let res = service
        .get_past_endorsees(&bill_id_test(), &identity.identity.node_id)
        .await
        .expect("call works");
    // caller is payee and drawer != drawee, there's only 1 holder before the caller (drawer)
    // even though it's the current holder after the last endorse block
    assert_eq!(res.len(), 1);

    // Caller = Endorsee
    let res = service
        .get_past_endorsees(&bill_id_test(), &endorse_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is endorsee and drawer != drawee, there are 2 holders before the caller (drawer, payee)
    // even though they were holder after mint
    assert_eq!(res.len(), 2);
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );

    // Caller = Sell Endorsee
    let res = service
        .get_past_endorsees(&bill_id_test(), &sell_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is sell_endorsee and drawer != drawee, there are 2 holders before the caller (drawer, payee)
    // that's because endorsee is anon
    assert_eq!(res.len(), 2);
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );

    // Caller = Mint
    let res = service
        .get_past_endorsees(&bill_id_test(), &mint_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is mint and drawer != drawee, there are 3 holders before the caller (drawer, payee, sell_endorsee)
    // that's because endorsee is anon
    assert_eq!(res.len(), 3);
    // sell_endorsee is the first one (latest timestamp)
    assert_eq!(
        res[0].pay_to_the_order_of.node_id,
        sell_endorsee_clone.node_id
    );
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );
}

#[tokio::test]
async fn get_past_endorsees_multi() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    let drawer = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let mint_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let endorse_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let sell_endorsee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));

    bill.drawer = drawer.clone();
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let drawer_clone = bill.drawer.clone();
    let endorse_endorsee_clone = endorse_endorsee.clone();
    let mint_endorsee_clone = mint_endorsee.clone();
    let sell_endorsee_clone = sell_endorsee.clone();

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now();
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add endorse block from payee to endorsee
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block));

            // add sell block from endorsee to sell endorsee
            let sell_block = BillBlock::create_block_for_sell(
                bill_id_test(),
                chain.get_latest_block(),
                &BillSellBlockData {
                    buyer: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    // endorsed by endorsee
                    seller: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 2,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 2,
            )
            .unwrap();
            assert!(chain.try_add_block(sell_block));

            // add mint block from sell endorsee to mint endorsee
            let mint_block = BillBlock::create_block_for_mint(
                bill_id_test(),
                chain.get_latest_block(),
                &BillMintBlockData {
                    endorsee: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    // endorsed by sell endorsee
                    endorser: BillParticipantBlockData::Ident(sell_endorsee.clone().into()),
                    sum: Sum::new_sat(15000).expect("sat works"),
                    signatory: None,
                    signing_timestamp: now + 3,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 3,
            )
            .unwrap();
            assert!(chain.try_add_block(mint_block));

            // add endorse block back to endorsee
            let endorse_block_back = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(mint_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 4,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 4,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block_back));

            // add endorse block back to payee (caller)
            let endorse_block_last = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorsee: BillParticipantBlockData::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity)
                            .unwrap()
                            .into(),
                    ),
                    // endorsed by payee
                    endorser: BillParticipantBlockData::Ident(endorse_endorsee.clone().into()),
                    signatory: None,
                    signing_timestamp: now + 5,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 5,
            )
            .unwrap();
            assert!(chain.try_add_block(endorse_block_last));

            Ok(chain)
        });
    let service = get_service(ctx);

    // Caller = Payee (=identity)
    let res = service
        .get_past_endorsees(&bill_id_test(), &identity.identity.node_id)
        .await
        .expect("call works");
    // caller is payee and drawer != drawee, there's only 1 holder before the caller (drawer)
    // even though it's the current holder after the last endorse block
    assert_eq!(res.len(), 1);

    // Caller = Endorsee
    let res = service
        .get_past_endorsees(&bill_id_test(), &endorse_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is endorsee and drawer != drawee, there are 2 holders before the caller (drawer, payee)
    // even though they were holder after mint
    assert_eq!(res.len(), 2);
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );

    // Caller = Sell Endorsee
    let res = service
        .get_past_endorsees(&bill_id_test(), &sell_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is sell_endorsee and drawer != drawee, there are 3 holders before the caller (drawer, payee, endorsee)
    assert_eq!(res.len(), 3);
    // endorsee is the first one (latest timestamp)
    assert_eq!(
        res[0].pay_to_the_order_of.node_id,
        endorse_endorsee_clone.node_id
    );
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );

    // Caller = Mint
    let res = service
        .get_past_endorsees(&bill_id_test(), &mint_endorsee_clone.node_id)
        .await
        .expect("call works");
    // caller is mint and drawer != drawee, there are 4 holders before the caller (drawer, payee, endorsee, sell_endorsee)
    assert_eq!(res.len(), 4);
    // sell_endorsee is the first one (latest timestamp)
    assert_eq!(
        res[0].pay_to_the_order_of.node_id,
        sell_endorsee_clone.node_id
    );
    // endorsee is the second one
    assert_eq!(
        res[1].pay_to_the_order_of.node_id,
        endorse_endorsee_clone.node_id
    );
    // payee is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == identity.identity.node_id),
    );
    // drawer is in the list
    assert!(
        res.iter()
            .map(|r| r.pay_to_the_order_of.node_id.clone())
            .any(|n| n == drawer_clone.node_id),
    );
}

#[tokio::test]
async fn past_payments_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());

    let identity_clone = identity.identity.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    // paid
    ctx.bill_store.expect_is_paid().returning(|_| Ok(true));

    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // req to pay
            assert!(chain.try_add_block(request_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block(),
                None,
            )));
            // paid
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            assert!(chain.try_add_block(sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
            )));
            // rejected
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            assert!(
                chain.try_add_block(reject_buy_block(&bill_id_test(), chain.get_latest_block(),))
            );
            // expired
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            // active
            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                Some(Timestamp::new(1931593928).unwrap()),
            )));

            Ok(chain)
        });

    let service = get_service(ctx);

    let res_past_payments = service
        .get_past_payments(
            &bill_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            Timestamp::new(1931593928).unwrap(),
        )
        .await;

    assert!(res_past_payments.is_ok());
    assert_eq!(res_past_payments.as_ref().unwrap().len(), 4);
    match res_past_payments.as_ref().unwrap()[0] {
        PastPaymentResult::Payment(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Paid(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[1] {
        PastPaymentResult::Sell(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Paid(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[2] {
        PastPaymentResult::Sell(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Rejected(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[3] {
        PastPaymentResult::Sell(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Expired(_)));
        }
        _ => panic!("wrong result"),
    };
}

#[tokio::test]
async fn past_payments_recourse() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());

    let identity_clone = identity.identity.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    // not paid
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // req to pay
            assert!(chain.try_add_block(request_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block(),
                None,
            )));
            // reject payment
            assert!(chain.try_add_block(reject_to_pay_block(
                &bill_id_test(),
                chain.get_latest_block(),
            )));
            // req to recourse
            assert!(chain.try_add_block(request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            // recourse - paid
            assert!(chain.try_add_block(recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
            )));
            // req to recourse
            assert!(chain.try_add_block(request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            // reject
            assert!(chain.try_add_block(reject_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
            )));
            // expired
            assert!(chain.try_add_block(request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));
            // active
            assert!(chain.try_add_block(request_to_recourse_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                Some(Timestamp::new(1931593928).unwrap()),
            )));

            Ok(chain)
        });

    let service = get_service(ctx);

    let res_past_payments = service
        .get_past_payments(
            &bill_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            Timestamp::new(1931593928).unwrap(),
        )
        .await;

    assert!(res_past_payments.is_ok());
    assert_eq!(res_past_payments.as_ref().unwrap().len(), 4);
    match res_past_payments.as_ref().unwrap()[0] {
        PastPaymentResult::Payment(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Rejected(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[1] {
        PastPaymentResult::Recourse(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Paid(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[2] {
        PastPaymentResult::Recourse(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Rejected(_)));
        }
        _ => panic!("wrong result"),
    };
    match res_past_payments.as_ref().unwrap()[3] {
        PastPaymentResult::Recourse(ref data) => {
            assert!(matches!(data.status, PaymentStatus::Expired(_)));
        }
        _ => panic!("wrong result"),
    };
}

#[tokio::test]
async fn reject_acceptance_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to accept block
            let req_to_accept = BillBlock::create_block_for_request_to_accept(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToAcceptBlockData {
                    requester: payee.clone().into(),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                    acceptance_deadline_timestamp: now + 1 + 2 * ACCEPT_DEADLINE_SECONDS,
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_accept));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::AcceptBill))
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);
    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectAcceptance,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity).unwrap()),
            &identity.key_pair,
            now + 2,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().blocks()[2].op_code,
        BillOpCode::RejectToAccept
    );
}

#[tokio::test]
async fn reject_acceptance_fails_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to accept block
            let req_to_accept = BillBlock::create_block_for_request_to_accept(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToAcceptBlockData {
                    requester: payee.clone().into(),
                    signatory: None,
                    signing_timestamp: now + 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                    acceptance_deadline_timestamp: now + 1 + 2 * ACCEPT_DEADLINE_SECONDS,
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now + 1,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_accept));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::AcceptBill))
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectAcceptance,
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity)),
            &identity.key_pair,
            now + 2,
        )
        .await;
    assert!(res.is_err());
    assert!(matches!(
        res.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SignerCantBeAnon
        ))
    ));
}

#[tokio::test]
async fn reject_buying_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());

    let identity_clone = identity.identity.clone();
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));

            Ok(chain)
        });

    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::BuyBill))
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectBuying,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().blocks()[2].op_code,
        BillOpCode::RejectToBuy
    );
}

#[tokio::test]
async fn reject_buying_anon_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());

    let identity_clone = identity.identity.clone();
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            assert!(chain.try_add_block(offer_to_sell_block(
                &bill_id_test(),
                chain.get_latest_block(),
                &BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                None,
            )));

            Ok(chain)
        });

    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::BuyBill))
        .returning(|_, _| Ok(()));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectBuying,
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().blocks()[2].op_code,
        BillOpCode::RejectToBuy
    );
}

#[tokio::test]
async fn reject_payment() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to pay
            let req_to_pay = BillBlock::create_block_for_request_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToPayBlockData {
                    requester: payee.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: bill.sum.clone(),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * PAYMENT_DEADLINE_SECONDS,
                    },
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_pay));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::PayBill))
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectPayment,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity).unwrap()),
            &identity.key_pair,
            now + 1,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().blocks()[2].op_code,
        BillOpCode::RejectToPay
    );
}

#[tokio::test]
async fn reject_payment_fails_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to pay
            let req_to_pay = BillBlock::create_block_for_request_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToPayBlockData {
                    requester: payee.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: bill.sum.clone(),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * PAYMENT_DEADLINE_SECONDS,
                    },
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_pay));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::PayBill))
        .returning(|_, _| Ok(()));
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectPayment,
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity)),
            &identity.key_pair,
            now + 1,
        )
        .await;
    assert!(res.is_err());
    assert!(matches!(
        res.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SignerCantBeAnon
        ))
    ));
}

#[tokio::test]
async fn reject_recourse() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to pay
            let req_to_pay = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(bill_identified_participant_only_node_id(
                        payee.node_id(),
                    ))
                    .into(),
                    recoursee: BillIdentParticipant::new(get_baseline_identity().identity)
                        .unwrap()
                        .into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_pay));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::RecourseBill))
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectPaymentForRecourse,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity).unwrap()),
            &identity.key_pair,
            now + 1,
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(
        res.as_ref().unwrap().blocks()[2].op_code,
        BillOpCode::RejectToPayRecourse
    );
}

#[tokio::test]
async fn reject_recourse_fails_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let bill = get_baseline_bill(&bill_id_test());
    let payee = bill.payee.clone();
    let now = Timestamp::now();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));

            // add req to pay
            let req_to_pay = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(bill_identified_participant_only_node_id(
                        payee.node_id(),
                    ))
                    .into(),
                    recoursee: BillIdentParticipant::new(get_baseline_identity().identity)
                        .unwrap()
                        .into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_pay));

            Ok(chain)
        });
    ctx.transport_service
        .expect_send_request_to_action_rejected_event()
        .with(always(), eq(ActionType::RecourseBill))
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RejectPaymentForRecourse,
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity)),
            &identity.key_pair,
            now + 1,
        )
        .await;
    assert!(res.is_err());
    assert!(matches!(
        res.as_ref().unwrap_err(),
        Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::SignerCantBeAnon
        ))
    ));
}

#[tokio::test]
async fn check_bills_in_recourse_payment_baseline() {
    let mut ctx = get_ctx();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(
        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
    );

    ctx.company_store.expect_get_all().returning(move || {
        let company = get_baseline_company_data();
        let map = HashMap::from([(
            company.0.clone(),
            (company.1.0.clone(), company.1.1.clone()),
        )]);
        Ok(map)
    });
    ctx.company_store.expect_get().returning(|_| {
        let mut company = get_baseline_company_data();
        company.1.0.id = node_id_test_another();
        Ok(company.1.0)
    });
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_identity_proof_test()]));

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store
        .expect_get_bill_ids_waiting_for_recourse_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_recourse_payment_state()
        .returning(|_, _, _| Ok(()));
    let recoursee = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now() - 10; // to avoid race with called code
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_recourse = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(
                        BillIdentParticipant::new(get_baseline_identity().identity).unwrap(),
                    )
                    .into(),
                    recoursee: bill_identified_participant_only_node_id(recoursee.clone()).into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: None,
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_recourse));
            Ok(chain)
        });
    ctx.transport_service
        .expect_send_bill_recourse_paid_event()
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block and company block
    expect_populates_company_and_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service.check_bills_in_recourse_payment().await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_bills_in_recourse_payment_company_is_recourser() {
    let mut ctx = get_ctx();
    let mut identity = get_baseline_identity();
    identity.key_pair = BcrKeys::new();
    identity.identity.node_id = NodeId::new(identity.key_pair.pub_key(), bitcoin::Network::Testnet);

    let company = get_baseline_company_data();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(BillIdentParticipant::from(company.1.0.clone()));

    ctx.company_store.expect_get().returning(|_| {
        let mut company = get_baseline_company_data();
        company.1.0.id = node_id_test_another();
        Ok(company.1.0)
    });
    ctx.company_store
        .expect_get_email_confirmations()
        .returning(|_| Ok(vec![signed_identity_proof_test()]));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store
        .expect_get_bill_ids_waiting_for_recourse_payment()
        .returning(|| Ok(vec![bill_id_test()]));
    ctx.bill_store
        .expect_set_recourse_payment_state()
        .returning(|_, _, _| Ok(()));
    let company_clone = company.clone();
    ctx.company_store.expect_get_all().returning(move || {
        let mut map = HashMap::new();
        map.insert(
            company_clone.0.clone(),
            (company_clone.1.0.clone(), company_clone.1.1.clone()),
        );
        Ok(map)
    });
    let company_clone = company.1.0.clone();
    let recoursee = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Testnet);
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let now = Timestamp::now() - 10; // to avoid race with called code
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_recourse = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(BillIdentParticipant::from(
                        company_clone.clone(),
                    ))
                    .into(),
                    recoursee: bill_identified_participant_only_node_id(recoursee.clone()).into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: now + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: Some(BillSignatoryBlockData {
                        node_id: get_baseline_identity().identity.node_id.clone(),
                        name: Some(get_baseline_identity().identity.name.clone()),
                    }),
                    signing_timestamp: now,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::from_private_key(&private_key_test()),
                Some(&BcrKeys::from_private_key(&private_key_test())),
                &BcrKeys::from_private_key(&private_key_test()),
                now,
            )
            .unwrap();
            assert!(chain.try_add_block(req_to_recourse));
            Ok(chain)
        });
    ctx.transport_service
        .expect_send_bill_recourse_paid_event()
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block and company block
    expect_populates_company_and_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service.check_bills_in_recourse_payment().await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn request_recourse_accept_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let payee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(payee.clone());
    let recoursee = payee.clone();
    let endorsee_caller = BillIdentParticipant::new(identity.identity.clone()).unwrap();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorser: bill.payee.clone().into(),
                    endorsee: BillParticipantBlockData::Ident(endorsee_caller.clone().into()),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(endorse_block);
            let req_to_accept = BillBlock::create_block_for_request_to_accept(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToAcceptBlockData {
                    requester: bill.payee.clone().into(),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                    acceptance_deadline_timestamp: test_ts() - 1 + 2 * ACCEPT_DEADLINE_SECONDS,
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(req_to_accept);
            let reject_accept = BillBlock::create_block_for_reject_to_accept(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRejectBlockData {
                    rejecter: bill.drawee.clone().into(),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: empty_address(),
                    signer_identity_proof: signed_identity_proof_test().into(),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(reject_accept);
            Ok(chain)
        });
    // Request to recourse event should be sent
    ctx.transport_service
        .expect_send_recourse_action_event()
        .returning(|_, _, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestRecourse(
                recoursee,
                RecourseReason::Accept,
                safe_deadline_ts(RECOURSE_DEADLINE_SECONDS),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 5);
    assert!(res.unwrap().blocks()[4].op_code == BillOpCode::RequestRecourse);
}

#[tokio::test]
async fn request_recourse_works_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let payee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(payee.clone());
    let recoursee = payee.clone();
    let endorsee_caller = BillIdentParticipant::new(identity.identity.clone()).unwrap();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorser: bill.payee.clone().into(),
                    endorsee: BillParticipantBlockData::Ident(endorsee_caller.clone().into()),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(endorse_block);
            let req_to_pay = BillBlock::create_block_for_request_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToPayBlockData {
                    requester: bill.payee.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: bill.sum.clone(),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * PAYMENT_DEADLINE_SECONDS,
                    },
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(req_to_pay);
            let reject_pay = BillBlock::create_block_for_reject_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRejectBlockData {
                    rejecter: bill.drawee.clone().into(),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: empty_address(),
                    signer_identity_proof: signed_identity_proof_test().into(),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(reject_pay);
            Ok(chain)
        });
    // Request to recourse event should be sent
    ctx.transport_service
        .expect_send_recourse_action_event()
        .returning(|_, _, _| Ok(()));
    // Populates identity block
    expect_populates_identity_block(&mut ctx);
    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestRecourse(
                recoursee,
                RecourseReason::Pay(Sum::new_sat(15000).expect("sat works")),
                safe_deadline_ts(RECOURSE_DEADLINE_SECONDS),
            ),
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 5);
    assert!(res.unwrap().blocks()[4].op_code == BillOpCode::RequestRecourse);
}

#[tokio::test]
async fn request_recourse_payment_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let payee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee = BillParticipant::Ident(payee.clone());
    let recoursee = payee.clone();
    let endorsee_caller = BillIdentParticipant::new(identity.identity.clone()).unwrap();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let endorse_block = BillBlock::create_block_for_endorse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillEndorseBlockData {
                    endorser: bill.payee.clone().into(),
                    endorsee: BillParticipantBlockData::Ident(endorsee_caller.clone().into()),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(endorse_block);
            let req_to_pay = BillBlock::create_block_for_request_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestToPayBlockData {
                    requester: bill.payee.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: bill.sum.clone(),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * PAYMENT_DEADLINE_SECONDS,
                    },
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(req_to_pay);
            let reject_pay = BillBlock::create_block_for_reject_to_pay(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRejectBlockData {
                    rejecter: bill.drawee.clone().into(),
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: empty_address(),
                    signer_identity_proof: signed_identity_proof_test().into(),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(reject_pay);
            Ok(chain)
        });
    // Request to recourse event should be sent
    ctx.transport_service
        .expect_send_recourse_action_event()
        .returning(|_, _, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::RequestRecourse(
                recoursee,
                RecourseReason::Pay(Sum::new_sat(15000).expect("sat works")),
                safe_deadline_ts(RECOURSE_DEADLINE_SECONDS),
            ),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().blocks().len() == 5);
    assert!(res.unwrap().blocks()[4].op_code == BillOpCode::RequestRecourse);
}

#[tokio::test]
async fn recourse_bitcredit_bill_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    let recoursee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let recoursee_clone = recoursee.clone();
    let identity_clone = identity.identity.clone();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_recourse = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(
                        BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                    )
                    .into(),
                    recoursee: recoursee_clone.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(req_to_recourse);
            Ok(chain)
        });
    // Recourse paid event should be sent
    ctx.transport_service
        .expect_send_bill_recourse_paid_event()
        .returning(|_, _| Ok(()));
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Recourse(recoursee),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().blocks().len(), 3);
    assert_eq!(res.unwrap().blocks()[2].op_code, BillOpCode::Recourse);
}

#[tokio::test]
async fn recourse_bitcredit_bill_works_for_anon() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    bill.payee =
        BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap());
    let recoursee = bill_identified_participant_only_node_id(NodeId::new(
        BcrKeys::new().pub_key(),
        bitcoin::Network::Testnet,
    ));
    let recoursee_clone = recoursee.clone();
    let identity_clone = identity.identity.clone();

    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_store.expect_is_paid().returning(|_| Ok(false));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            let req_to_recourse = BillBlock::create_block_for_request_recourse(
                bill_id_test(),
                chain.get_latest_block(),
                &BillRequestRecourseBlockData {
                    recourser: BillParticipant::Ident(
                        BillIdentParticipant::new(identity_clone.clone()).unwrap(),
                    )
                    .into(),
                    recoursee: recoursee_clone.clone().into(),
                    payment_data: BillPaymentBlockData {
                        sum: Sum::new_sat(15000).expect("sat works"),
                        payment_address: valid_payment_address_testnet(),
                        payment_deadline: test_ts() - 1 + 2 * RECOURSE_DEADLINE_SECONDS,
                    },
                    recourse_reason: BillRecourseReasonBlockData::Pay,
                    signatory: None,
                    signing_timestamp: test_ts() - 1,
                    signing_address: Some(empty_address()),
                    signer_identity_proof: Some(signed_identity_proof_test().into()),
                },
                &BcrKeys::new(),
                None,
                &BcrKeys::from_private_key(&private_key_test()),
                test_ts() - 1,
            )
            .unwrap();
            chain.try_add_block(req_to_recourse);
            Ok(chain)
        });
    // Recourse paid event should be sent
    ctx.transport_service
        .expect_send_bill_recourse_paid_event()
        .returning(|_, _| Ok(()));

    // Populates identity block
    expect_populates_identity_block(&mut ctx);

    let service = get_service(ctx);

    let res = service
        .execute_bill_action(
            &bill_id_test(),
            BillAction::Recourse(recoursee),
            &BillParticipant::Anon(BillAnonParticipant::new(identity.identity.clone())),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().blocks().len(), 3);
    assert_eq!(res.unwrap().blocks()[2].op_code, BillOpCode::Recourse);
}

#[test]
fn check_req_for_expiration_baseline() {
    let ctx = get_ctx();
    let service = get_service(ctx);
    let mut bill_payment = get_baseline_cached_bill(bill_id_test());
    bill_payment.status.payment = BillPaymentStatus {
        time_of_request_to_pay: Some(Timestamp::new(1531593928).unwrap()),
        requested_to_pay: true,
        paid: false,
        request_to_pay_timed_out: false,
        rejected_to_pay: false,
        payment_deadline_timestamp: Some(
            Timestamp::new(1531593928).unwrap() + PAYMENT_DEADLINE_SECONDS,
        ),
    };
    bill_payment.status.is_mature = true;

    // true, because payment deadline was several days ago
    assert!(
        service
            .check_requests_for_expiration(&bill_payment, test_ts())
            .unwrap()
    );
    bill_payment.status.payment.request_to_pay_timed_out = true;
    // if it already timed out, we don't need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, test_ts())
            .unwrap()
    );
    bill_payment.status.payment.request_to_pay_timed_out = false;
    // false, because time is before the deadline
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1431593928).unwrap())
            .unwrap()
    );
    bill_payment.data.maturity_date = Date::new("2018-07-15").unwrap();
    bill_payment.status.is_mature = false;
    // false, because not mature and before maturity date
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531593929).unwrap())
            .unwrap()
    );
    // true, because on maturity date and bill was not mature before
    assert!(
        service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531653929).unwrap())
            .unwrap()
    );
    // true, because exactly maturity date start of day (00:00:00)
    assert!(
        service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531612800).unwrap())
            .unwrap()
    );
    // false, because exactly one second before maturity date start of day (23:59:59)
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531612799).unwrap())
            .unwrap()
    );
    // true, because after maturity date and bill was not mature before
    assert!(
        service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531978728).unwrap())
            .unwrap()
    );
    bill_payment.status.is_mature = true;
    bill_payment.current_waiting_state = Some(BillCurrentWaitingState::Payment(
        BillWaitingForPaymentState {
            payer: empty_bill_identified_participant(),
            payee: BillParticipant::Ident(empty_bill_identified_participant()),
            payment_data: BillWaitingStatePaymentData {
                time_of_request: Timestamp::new(1531593928).unwrap(),
                sum: Sum::new_sat(10).expect("sat works"),
                address_to_pay: valid_payment_address_testnet(),
                tx_id: None,
                confirmations: 0,
                in_mempool: false,
                payment_deadline: Some(
                    Timestamp::new(1531593928).unwrap() + 2 * PAYMENT_DEADLINE_SECONDS,
                ),
            },
        },
    ));
    // req to pay expired, but not yet 2 days after end of day maturity date
    // current waiting state set, so wasnt recalculated yet
    assert!(
        service
            .check_requests_for_expiration(
                &bill_payment,
                Timestamp::new(1531593929).unwrap() + PAYMENT_DEADLINE_SECONDS + 1
            )
            .unwrap()
    );
    bill_payment.current_waiting_state = None;
    // after req to pay, and after end of day maturity date, payment expired
    assert!(
        service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1831593928).unwrap())
            .unwrap()
    );
    // 1 sec after req to pay, not expired at all
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1531593929).unwrap())
            .unwrap()
    );
    bill_payment.status.payment.rejected_to_pay = true;
    // already rejected, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1831593928).unwrap())
            .unwrap()
    );
    bill_payment.status.payment.rejected_to_pay = false;
    bill_payment.status.payment.paid = true;
    // already paid, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_payment, Timestamp::new(1831593928).unwrap())
            .unwrap()
    );
    bill_payment.status.payment.paid = false;

    let mut bill_acceptance = get_baseline_cached_bill(bill_id_test());
    bill_acceptance.status.acceptance = BillAcceptanceStatus {
        time_of_request_to_accept: Some(Timestamp::new(1531593928).unwrap()),
        requested_to_accept: true,
        accepted: false,
        request_to_accept_timed_out: false,
        rejected_to_accept: false,
        acceptance_deadline_timestamp: Some(
            Timestamp::new(1531593928).unwrap() + ACCEPT_DEADLINE_SECONDS,
        ),
    };
    bill_acceptance.status.is_mature = true;

    assert!(
        service
            .check_requests_for_expiration(&bill_acceptance, test_ts())
            .unwrap()
    );
    bill_acceptance
        .status
        .acceptance
        .request_to_accept_timed_out = true;
    // already expired, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_acceptance, test_ts())
            .unwrap()
    );
    bill_acceptance
        .status
        .acceptance
        .request_to_accept_timed_out = false;
    bill_acceptance.status.acceptance.rejected_to_accept = true;
    // already rejected, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_acceptance, test_ts())
            .unwrap()
    );
    bill_acceptance.status.acceptance.rejected_to_accept = false;
    bill_acceptance.status.acceptance.accepted = true;
    // already accepted, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_acceptance, test_ts())
            .unwrap()
    );
    bill_acceptance.status.acceptance.accepted = false;

    let mut bill_sell = get_baseline_cached_bill(bill_id_test());
    bill_sell.status.sell = BillSellStatus {
        time_of_last_offer_to_sell: Some(Timestamp::new(1531593928).unwrap()),
        offered_to_sell: true,
        sold: false,
        offer_to_sell_timed_out: false,
        rejected_offer_to_sell: false,
        buying_deadline_timestamp: Some(Timestamp::new(1531593928).unwrap() + DAY_IN_SECS),
    };
    bill_sell.status.is_mature = true;

    assert!(
        service
            .check_requests_for_expiration(&bill_sell, test_ts())
            .unwrap()
    );

    bill_sell.status.sell.offer_to_sell_timed_out = true;
    // already expired, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_sell, test_ts())
            .unwrap()
    );
    bill_sell.status.sell.offer_to_sell_timed_out = false;
    bill_sell.status.sell.rejected_offer_to_sell = true;
    // already rejected, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_sell, test_ts())
            .unwrap()
    );
    bill_sell.status.sell.rejected_offer_to_sell = false;
    bill_sell.status.sell.sold = true;
    // already sold, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_sell, test_ts())
            .unwrap()
    );
    bill_sell.status.sell.sold = false;

    let mut bill_recourse = get_baseline_cached_bill(bill_id_test());
    bill_recourse.status.recourse = BillRecourseStatus {
        time_of_last_request_to_recourse: Some(Timestamp::new(1531593928).unwrap()),
        requested_to_recourse: true,
        recoursed: false,
        request_to_recourse_timed_out: false,
        rejected_request_to_recourse: false,
        recourse_deadline_timestamp: Some(
            Timestamp::new(1531593928).unwrap() + RECOURSE_DEADLINE_SECONDS,
        ),
    };
    bill_recourse.status.is_mature = true;

    assert!(
        service
            .check_requests_for_expiration(&bill_recourse, test_ts())
            .unwrap()
    );
    bill_recourse.status.recourse.request_to_recourse_timed_out = true;
    // already expired, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_recourse, test_ts())
            .unwrap()
    );
    bill_recourse.status.recourse.rejected_request_to_recourse = true;
    // already rejected, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_recourse, test_ts())
            .unwrap()
    );
    bill_recourse.status.recourse.rejected_request_to_recourse = false;
    bill_recourse.status.recourse.recoursed = true;
    // already recoursed, no need to check anymore
    assert!(
        !service
            .check_requests_for_expiration(&bill_recourse, test_ts())
            .unwrap()
    );
    bill_recourse.status.recourse.recoursed = false;
}

#[tokio::test]
async fn req_to_mint_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            chain.try_add_block(accept_block(&bill.id, chain.get_latest_block()));
            Ok(chain)
        });
    ctx.mint_store
        .expect_get_requests()
        .returning(|_, _, _| Ok(vec![]));
    ctx.mint_client
        .expect_enquire_mint_quote()
        .returning(|_, _, _| Ok(get_uuid_v4()));
    ctx.mint_store
        .expect_add_request()
        .returning(|_, _, _, _, _| Ok(()));
    // Asset request to mint event is sent
    ctx.transport_service
        .expect_send_request_to_mint_event()
        .returning(|_, _, _| Ok(()));
    ctx.transport_service.expect_on_contact_transport(|t| {
        t.expect_resolve_contact().returning(|_| Ok(None));
    });

    let service = get_service(ctx);

    let res = service
        .request_to_mint(
            &bill_id_test(),
            &node_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn get_mint_state_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_store.expect_get_offer().returning(|_| {
        Ok(Some(MintOffer {
            mint_request_id: get_uuid_v4(),
            keyset_id: "keyset_id".to_owned(),
            expiration_timestamp: test_ts(),
            discounted_sum: Sum::new_sat(1500).unwrap(),
            proofs: None,
            proofs_spent: false,
            recovery_data: None,
        }))
    });

    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(|_, _| {
            Ok(vec![
                MintRequest {
                    requester_node_id: node_id_test(),
                    bill_id: bill_id_test(),
                    mint_node_id: node_id_test_other(),
                    mint_request_id: get_uuid_v4(),
                    timestamp: test_ts(),
                    status: MintRequestStatus::Pending,
                },
                MintRequest {
                    requester_node_id: node_id_test(),
                    bill_id: bill_id_test(),
                    mint_node_id: node_id_test_other(),
                    mint_request_id: get_uuid_v4(),
                    timestamp: test_ts(),
                    status: MintRequestStatus::Offered,
                },
                MintRequest {
                    requester_node_id: node_id_test(),
                    bill_id: bill_id_test(),
                    mint_node_id: node_id_test_other(),
                    mint_request_id: get_uuid_v4(),
                    timestamp: test_ts(),
                    status: MintRequestStatus::MintingEnabled,
                },
                MintRequest {
                    requester_node_id: node_id_test(),
                    bill_id: bill_id_test(),
                    mint_node_id: node_id_test_other(),
                    mint_request_id: get_uuid_v4(),
                    timestamp: test_ts(),
                    status: MintRequestStatus::Accepted,
                },
            ])
        });

    let service = get_service(ctx);
    let res = service
        .get_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().len(), 4);
    for mrs in res.as_ref().unwrap() {
        match mrs.request.status {
            MintRequestStatus::Offered
            | MintRequestStatus::Accepted
            | MintRequestStatus::MintingEnabled => {
                assert!(mrs.offer.is_some());
            }
            _ => (),
        }
    }
}

#[tokio::test]
async fn cancel_mint_state_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_cancel_quote_for_mint()
        .returning(|_, _| Ok(()));
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store.expect_get_request().returning(move |_| {
        Ok(Some(MintRequest {
            requester_node_id: req_node_id.clone(),
            bill_id: bill_id_test(),
            mint_node_id: node_id_test(),
            mint_request_id: get_uuid_v4(),
            timestamp: test_ts(),
            status: MintRequestStatus::Pending,
        }))
    });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .cancel_request_to_mint(&get_uuid_v4(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn accept_mint_offer_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let mut identity = IdentityWithAll {
        identity: empty_identity(),
        key_pair: BcrKeys::new(),
    };
    identity.identity.node_id = NodeId::new(identity.key_pair.pub_key(), bitcoin::Network::Testnet);
    identity.identity.postal_address.country = Some(Country::AT);
    identity.identity.postal_address.city = Some(City::new("Vienna").unwrap());
    identity.identity.postal_address.address = Some(Address::new("Hayekweg 5").unwrap());

    let mut bill = get_baseline_bill(&bill_id_test());
    bill.payee = BillParticipant::Ident(bill_identified_participant_only_node_id(
        identity.identity.node_id.clone(),
    ));
    ctx.bill_store
        .expect_save_bill_to_cache()
        .returning(|_, _, _| Ok(()));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| {
            let mut chain = get_genesis_chain(Some(bill.clone()));
            chain.try_add_block(accept_block(&bill.id, chain.get_latest_block()));
            Ok(chain)
        });

    // Populate identity block
    expect_populates_identity_block(&mut ctx);

    ctx.mint_client
        .expect_resolve_quote_for_mint()
        .returning(|_, _, _| Ok(()));
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store.expect_get_request().returning(move |_| {
        Ok(Some(MintRequest {
            requester_node_id: req_node_id.clone(),
            bill_id: bill_id_test(),
            mint_node_id: node_id_test(),
            mint_request_id: get_uuid_v4(),
            timestamp: Timestamp::new(1731593938).unwrap(),
            status: MintRequestStatus::Offered,
        }))
    });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(|_, _| Ok(vec![]));
    ctx.mint_store.expect_get_offer().returning(|_| {
        Ok(Some(MintOffer {
            mint_request_id: get_uuid_v4(),
            keyset_id: "keyset_id".to_owned(),
            expiration_timestamp: Timestamp::new(1731593938).unwrap(),
            discounted_sum: Sum::new_sat(1500).unwrap(),
            proofs: None,
            proofs_spent: false,
            recovery_data: None,
        }))
    });
    ctx.identity_store
        .expect_get_email_confirmations()
        .returning(|| Ok(vec![signed_identity_proof_test()]));
    // Asset request to mint event is sent
    ctx.transport_service
        .expect_send_bill_is_endorsed_event()
        .returning(|_| Ok(()));
    ctx.transport_service.expect_on_contact_transport(|t| {
        t.expect_resolve_contact().returning(|_| Ok(None));
    });

    let service = get_service(ctx);
    let res = service
        .accept_mint_offer(
            &get_uuid_v4(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            Timestamp::new(1731593930).unwrap(),
        )
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn reject_mint_offer_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_resolve_quote_for_mint()
        .returning(|_, _, _| Ok(()));
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store.expect_get_request().returning(move |_| {
        Ok(Some(MintRequest {
            requester_node_id: req_node_id.clone(),
            bill_id: bill_id_test(),
            mint_node_id: node_id_test(),
            mint_request_id: get_uuid_v4(),
            timestamp: test_ts(),
            status: MintRequestStatus::Offered,
        }))
    });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .reject_mint_offer(&get_uuid_v4(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_for_all_bills_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_lookup_quote_for_mint()
        .returning(|_, _| {
            Ok(QuoteStatusReply::Denied {
                tstamp: DateTimeUtc::UNIX_EPOCH,
            })
        });
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store
        .expect_get_all_active_requests()
        .returning(move || {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::Offered,
            }])
        });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service.check_mint_state_for_all_bills().await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_baseline() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_lookup_quote_for_mint()
        .returning(|_, _| {
            Ok(QuoteStatusReply::Denied {
                tstamp: DateTimeUtc::UNIX_EPOCH,
            })
        });
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::Offered,
            }])
        });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_pending_accepted() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_lookup_quote_for_mint()
        .returning(|_, _| {
            Ok(QuoteStatusReply::Accepted {
                keyset_id: cdk02::Id::try_from("00c7b45973e5f0fc".to_owned()).unwrap(),
            })
        });
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::Pending,
            }])
        });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_pending_offered() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_lookup_quote_for_mint()
        .returning(|_, _| {
            Ok(QuoteStatusReply::Offered {
                keyset_id: cdk02::Id::try_from("00c7b45973e5f0fc".to_owned()).unwrap(),
                expiration_date: DateTimeUtc::UNIX_EPOCH,
                discounted: bitcoin::Amount::default(),
            })
        });
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::Pending,
            }])
        });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));
    ctx.mint_store
        .expect_add_offer()
        .returning(|_, _, _, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_offered_accepted() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    ctx.mint_client
        .expect_lookup_quote_for_mint()
        .returning(|_, _| {
            Ok(QuoteStatusReply::Accepted {
                keyset_id: cdk02::Id::try_from("00c7b45973e5f0fc".to_owned()).unwrap(),
            })
        });
    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::Offered,
            }])
        });
    ctx.mint_store
        .expect_update_request()
        .returning(|_, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_minting_enabled_proofs() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    let req_node_id = identity.identity.node_id.clone();
    ctx.company_store.expect_get_all().returning(move || {
        let company = get_baseline_company_data();
        let mut map = HashMap::new();
        map.insert(
            company.0.clone(),
            (company.1.0.clone(), company.1.1.clone()),
        );
        Ok(map)
    });
    ctx.mint_client.expect_get_keyset_info().returning(|_, _| {
        Ok(ecash::KeySet {
            id: cdk02::Id::try_from("00c7b45973e5f0fc".to_owned()).unwrap(),
            unit: bcr_common::cashu::CurrencyUnit::Sat,
            keys: bcr_common::cashu::Keys::new(std::collections::BTreeMap::default()),
            final_expiry: None,
            active: Some(true),
            input_fee_ppk: 0,
        })
    });
    ctx.mint_client
        .expect_mint()
        .returning(|_, _, _, _, _, _, _, _| Ok("proofs".into()));
    ctx.mint_store
        .expect_add_recovery_data_to_offer()
        .returning(|_, _, _| Ok(()));
    ctx.mint_store
        .expect_add_proofs_to_offer()
        .returning(|_, _| Ok(()));
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::MintingEnabled,
            }])
        });
    ctx.mint_store.expect_get_offer().returning(|_| {
        Ok(Some(MintOffer {
            mint_request_id: get_uuid_v4(),
            keyset_id: "keyset_id".to_owned(),
            expiration_timestamp: Timestamp::new(1731593938).unwrap(),
            discounted_sum: Sum::new_sat(1500).unwrap(),
            proofs: None,
            proofs_spent: false,
            recovery_data: None,
        }))
    });

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn check_mint_state_minting_enabled_check_spent() {
    init_test_cfg();
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();

    let req_node_id = identity.identity.node_id.clone();
    ctx.mint_client.expect_get_keyset_info().returning(|_, _| {
        Ok(ecash::KeySet {
            id: cdk02::Id::try_from("00c7b45973e5f0fc".to_owned()).unwrap(),
            unit: bcr_common::cashu::CurrencyUnit::Sat,
            keys: bcr_common::cashu::Keys::new(std::collections::BTreeMap::default()),
            final_expiry: None,
            active: Some(true),
            input_fee_ppk: 0,
        })
    });
    ctx.mint_client
        .expect_check_if_proofs_are_spent()
        .returning(|_, _, _| Ok(true));
    ctx.mint_store
        .expect_set_proofs_to_spent_for_offer()
        .returning(|_| Ok(()));
    ctx.mint_store
        .expect_get_requests_for_bill()
        .returning(move |_, _| {
            Ok(vec![MintRequest {
                requester_node_id: req_node_id.clone(),
                bill_id: bill_id_test(),
                mint_node_id: node_id_test(),
                mint_request_id: get_uuid_v4(),
                timestamp: test_ts(),
                status: MintRequestStatus::MintingEnabled,
            }])
        });
    ctx.mint_store.expect_get_offer().returning(|_| {
        Ok(Some(MintOffer {
            mint_request_id: get_uuid_v4(),
            keyset_id: "keyset_id".to_owned(),
            expiration_timestamp: Timestamp::new(1731593938).unwrap(),
            discounted_sum: Sum::new_sat(1500).unwrap(),
            proofs: Some("proofs".into()),
            proofs_spent: false,
            recovery_data: None,
        }))
    });

    let service = get_service(ctx);
    let res = service
        .check_mint_state(&bill_id_test(), &identity.identity.node_id)
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn test_share_bill_with_court() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(|_| Ok(get_genesis_chain(None)));
    ctx.court_client
        .expect_share_with_court()
        .returning(|_, _, _| Ok(()));

    let service = get_service(ctx);
    let res = service
        .share_bill_with_court(
            &bill_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            &node_id_test_other(),
        )
        .await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn wrong_network_failures() {
    let participant =
        BillParticipant::Ident(bill_identified_participant_only_node_id(node_id_test()));
    let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
    let mainnet_participant = BillParticipant::Ident(bill_identified_participant_only_node_id(
        mainnet_node_id.clone(),
    ));
    let mainnet_bill_id = BillId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
    let identity = get_baseline_identity();
    let ctx = get_ctx();
    let service = get_service(ctx);

    assert!(matches!(
        service
            .get_combined_bitcoin_keys_for_bill(&mainnet_bill_id, &participant, &BcrKeys::new())
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_combined_bitcoin_keys_for_bill(
                &bill_id_test(),
                &mainnet_participant,
                &BcrKeys::new()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .get_detail(
                &mainnet_bill_id,
                &identity.identity,
                &participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_detail(
                &bill_id_test(),
                &identity.identity,
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service.get_bill_keys(&mainnet_bill_id).await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));

    assert!(matches!(
        service
            .check_payment_for_bill(&mainnet_bill_id, &identity.identity)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .check_offer_to_sell_payment_for_bill(&mainnet_bill_id, &identity)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .check_recourse_payment_for_bill(&mainnet_bill_id, &identity)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_past_endorsees(&mainnet_bill_id, &node_id_test())
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_past_endorsees(&bill_id_test(), &mainnet_node_id)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .get_past_payments(&mainnet_bill_id, &participant, &BcrKeys::new(), test_ts())
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_past_payments(
                &bill_id_test(),
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .get_endorsements(
                &mainnet_bill_id,
                &identity.identity,
                &participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_endorsements(
                &bill_id_test(),
                &identity.identity,
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .request_to_mint(
                &mainnet_bill_id,
                &node_id_test(),
                &participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .request_to_mint(
                &bill_id_test(),
                &mainnet_node_id,
                &participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .request_to_mint(
                &bill_id_test(),
                &node_id_test(),
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .get_mint_state(&mainnet_bill_id, &node_id_test())
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .get_mint_state(&bill_id_test(), &mainnet_node_id)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .cancel_request_to_mint(&get_uuid_v4(), &mainnet_node_id)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .check_mint_state(&mainnet_bill_id, &node_id_test())
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .check_mint_state(&bill_id_test(), &mainnet_node_id)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .accept_mint_offer(
                &get_uuid_v4(),
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .reject_mint_offer(&get_uuid_v4(), &mainnet_node_id)
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    // issue
    assert!(matches!(
        service
            .issue_new_bill(BillIssueData {
                t: 2,
                country_of_issuing: Country::GB,
                city_of_issuing: City::new("London").unwrap(),
                issue_date: Date::new("2030-01-01").unwrap(),
                maturity_date: Date::new("2030-04-01").unwrap(),
                drawee: mainnet_node_id.clone(),
                payee: node_id_test(),
                sum: Sum::new_sat(100).expect("sat works"),
                country_of_payment: Country::AT,
                city_of_payment: City::new("Vienna").unwrap(),
                file_upload_ids: vec![get_uuid_v4()],
                drawer_public_data: participant.clone(),
                drawer_keys: BcrKeys::new(),
                timestamp: test_ts(),
                blank_issue: false,
            })
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .issue_new_bill(BillIssueData {
                t: 2,
                country_of_issuing: Country::GB,
                city_of_issuing: City::new("London").unwrap(),
                issue_date: Date::new("2030-01-01").unwrap(),
                maturity_date: Date::new("2030-04-01").unwrap(),
                drawee: node_id_test(),
                payee: mainnet_node_id.clone(),
                sum: Sum::new_sat(100).expect("sat works"),
                country_of_payment: Country::AT,
                city_of_payment: City::new("Vienna").unwrap(),
                file_upload_ids: vec![get_uuid_v4()],
                drawer_public_data: participant.clone(),
                drawer_keys: BcrKeys::new(),
                timestamp: test_ts(),
                blank_issue: false,
            })
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .issue_new_bill(BillIssueData {
                t: 2,
                country_of_issuing: Country::GB,
                city_of_issuing: City::new("London").unwrap(),
                issue_date: Date::new("2030-01-01").unwrap(),
                maturity_date: Date::new("2030-04-01").unwrap(),
                drawee: node_id_test(),
                payee: node_id_test(),
                sum: Sum::new_sat(100).expect("sat works"),
                country_of_payment: Country::AT,
                city_of_payment: City::new("Vienna").unwrap(),
                file_upload_ids: vec![get_uuid_v4()],
                drawer_public_data: mainnet_participant.clone(),
                drawer_keys: BcrKeys::new(),
                timestamp: test_ts(),
                blank_issue: false,
            })
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    // execute bill action
    assert!(matches!(
        service
            .execute_bill_action(
                &mainnet_bill_id,
                BillAction::Accept,
                &participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .execute_bill_action(
                &bill_id_test(),
                BillAction::Accept,
                &mainnet_participant,
                &BcrKeys::new(),
                test_ts()
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
    assert!(matches!(
        service
            .share_bill_with_court(
                &mainnet_bill_id,
                &participant,
                &BcrKeys::new(),
                &node_id_test_other(),
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidBillId
        )))
    ));
    assert!(matches!(
        service
            .share_bill_with_court(
                &bill_id_test(),
                &mainnet_participant,
                &BcrKeys::new(),
                &node_id_test_other(),
            )
            .await,
        Err(Error::Validation(ValidationError::Protocol(
            ProtocolValidationError::InvalidNodeId
        )))
    ));
}

#[tokio::test]
async fn get_bill_history_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    let drawer_node_id = bill.drawer.node_id.clone();
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(get_genesis_chain(Some(bill.clone()))));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_bill_history(
            &bill_id_test(),
            &identity.identity,
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
            &identity.key_pair,
            test_ts(),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().blocks.len(), 1);
    assert_eq!(
        res.as_ref().unwrap().blocks[0].block_type,
        BillOpCode::Issue
    );
    assert_eq!(
        res.as_ref().unwrap().blocks[0].signed.data.node_id(),
        drawer_node_id
    );
}

#[tokio::test]
async fn get_local_bill_chain_baseline() {
    let mut ctx = get_ctx();
    let identity = get_baseline_identity();
    let mut bill = get_baseline_bill(&bill_id_test());
    bill.drawee = bill_identified_participant_only_node_id(identity.identity.node_id.clone());
    ctx.bill_store.expect_exists().returning(|_| Ok(true));
    let chain = get_genesis_chain(Some(bill.clone()));
    let chain_clone = chain.clone();
    ctx.bill_blockchain_store
        .expect_get_chain()
        .returning(move |_| Ok(chain_clone.clone()));
    ctx.transport_service.expect_on_notification_transport(|t| {
        t.expect_get_active_bill_notification()
            .with(eq(bill_id_test()))
            .returning(|_| None);
    });

    let res = get_service(ctx)
        .get_local_bill_chain(
            &bill_id_test(),
            &BillParticipant::Ident(BillIdentParticipant::new(identity.identity.clone()).unwrap()),
        )
        .await;
    assert!(res.is_ok());
    assert_eq!(res.as_ref().unwrap().blocks(), chain.blocks());
}
