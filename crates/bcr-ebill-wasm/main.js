import * as wasm from '../pkg/index.js';

// file upload
document.getElementById("fileInput").addEventListener("change", uploadFile);
document.getElementById("attach_identity_profile_picture").addEventListener("click", attachIdentityProfilePicture);
document.getElementById("attach_identity_document").addEventListener("click", attachIdentityDocument);
document.getElementById("attach_contact_avatar").addEventListener("click", attachContactAvatar);
document.getElementById("attach_company_proof").addEventListener("click", attachCompanyProof);
document.getElementById("remove_company_proof").addEventListener("click", removeCompanyProof);
document.getElementById("build_blossom_url").addEventListener("click", buildBlossomUrl);
document.getElementById("open_blossom_url").addEventListener("click", openBlossomUrl);

// notifs
document.getElementById("notif").addEventListener("click", triggerNotif);
document.getElementById("get_active_notif_status").addEventListener("click", getActiveNotif);
document.getElementById("get_notif_list").addEventListener("click", getNotifList);
document.getElementById("get_email_notifications_preferences_link").addEventListener("click", get_email_notifications_preferences_link);

// contacts
document.getElementById("contact_test").addEventListener("click", triggerContact);
document.getElementById("contact_test_anon").addEventListener("click", triggerAnonContact);
document.getElementById("fetch_contacts").addEventListener("click", fetchContacts);
document.getElementById("remove_contact_avatar").addEventListener("click", removeContactAvatar);
document.getElementById("edit_contact").addEventListener("click", editContact);
document.getElementById("delete_contact").addEventListener("click", deleteContact);
document.getElementById("fetch_temp").addEventListener("click", fetchTempFile);
document.getElementById("fetch_contact_file").addEventListener("click", fetchContactFile);
document.getElementById("search_contacts").addEventListener("click", searchContacts);

// identity
document.getElementById("switch_identity").addEventListener("click", switchIdentity);
document.getElementById("dev_mode_get_identity_chain").addEventListener("click", devModeGetIdentityChain);
document.getElementById("sync_identity_chain").addEventListener("click", syncIdentityChain);
document.getElementById("confirm_email").addEventListener("click", confirmEmail);
document.getElementById("verify_email").addEventListener("click", verifyEmail);
document.getElementById("change_email").addEventListener("click", changeEmail);
document.getElementById("edit_identity").addEventListener("click", editIdentity);
document.getElementById("get_confirmations").addEventListener("click", getIdentityConfirmations);
document.getElementById("get_identity").addEventListener("click", getIdentity);
document.getElementById("change_name").addEventListener("click", changeName);

// contact share
document.getElementById("share_contact_to").addEventListener("click", shareContact);
document.getElementById("share_company_contact_to").addEventListener("click", shareCompanyContact);
document.getElementById("list_pending_contact_shares").addEventListener("click", listPendingContactShares);
document.getElementById("get_pending_contact_share").addEventListener("click", getPendingContactShare);
document.getElementById("approve_contact_share").addEventListener("click", approveContactShare);
document.getElementById("approve_contact_share_with_share_back").addEventListener("click", approveContactShareWithShareBack);
document.getElementById("approve_contact_share_only_share_back").addEventListener("click", approveContactShareOnlyShareBack);
document.getElementById("reject_contact_share").addEventListener("click", rejectContactShare);

// bill actions
document.getElementById("bill_fetch_detail").addEventListener("click", fetchBillDetail);
document.getElementById("bill_fetch_endorsements").addEventListener("click", fetchBillEndorsements);
document.getElementById("bill_fetch_past_endorsees").addEventListener("click", fetchBillPastEndorsees);
document.getElementById("bill_fetch_past_payments").addEventListener("click", fetchBillPastPayments);
document.getElementById("bill_fetch_bill_file").addEventListener("click", fetchBillFile);
document.getElementById("bill_fetch_bills").addEventListener("click", fetchBillBills);
document.getElementById("bill_balances").addEventListener("click", fetchBillBalances);
document.getElementById("bill_search").addEventListener("click", fetchBillSearch);
document.getElementById("bill_history").addEventListener("click", fetchBillHistory);
document.getElementById("endorse_bill").addEventListener("click", endorseBill);
document.getElementById("blank_endorse_bill").addEventListener("click", endorseBillBlank);
document.getElementById("req_to_accept_bill").addEventListener("click", requestToAcceptBill);
document.getElementById("accept_bill").addEventListener("click", acceptBill);
document.getElementById("req_to_pay_bill").addEventListener("click", requestToPayBill);
document.getElementById("offer_to_sell_bill").addEventListener("click", offerToSellBill);
document.getElementById("req_to_recourse_bill").addEventListener("click", requestToRecourseBill);
document.getElementById("req_to_recourse_bill_payment").addEventListener("click", requestToRecourseBillPayment);
document.getElementById("reject_accept").addEventListener("click", rejectAcceptBill);
document.getElementById("reject_pay").addEventListener("click", rejectPayBill);
document.getElementById("reject_buying").addEventListener("click", rejectBuyingBill);
document.getElementById("reject_recourse").addEventListener("click", rejectRecourseBill);
document.getElementById("request_to_mint").addEventListener("click", requestToMint);
document.getElementById("get_mint_state").addEventListener("click", getMintState);
document.getElementById("check_mint_state").addEventListener("click", checkMintState);
document.getElementById("cancel_req_to_mint").addEventListener("click", cancelRegToMint);
document.getElementById("accept_mint_offer").addEventListener("click", acceptMintOffer);
document.getElementById("reject_mint_offer").addEventListener("click", rejectMintOffer);
document.getElementById("bill_test_self_drafted").addEventListener("click", triggerBill.bind(null, 1, false));
document.getElementById("bill_test_promissory").addEventListener("click", triggerBill.bind(null, 0, false));
document.getElementById("bill_test_promissory_blank").addEventListener("click", triggerBill.bind(null, 0, true));
document.getElementById("clear_bill_cache").addEventListener("click", clearBillCache);
document.getElementById("bitcoin_keys").addEventListener("click", getBitcoinKeys);
document.getElementById("sync_bill_chain").addEventListener("click", syncBillChain);
document.getElementById("sync_bill_chain_from_nostr").addEventListener("click", syncBillChainFromNostr);
document.getElementById("dev_mode_get_bill_chain").addEventListener("click", devModeGetBillChain);
document.getElementById("share_bill_with_court").addEventListener("click", shareBillWithCourt);
document.getElementById("mempool_link").addEventListener("click", mempoolLink);
document.getElementById("link_to_pay").addEventListener("click", linkToPay);
document.getElementById("check_and_estimate_sweep").addEventListener("click", checkAndEstimateSweep);
document.getElementById("sweep_funds").addEventListener("click", sweepFunds);

// companies
document.getElementById("company_create").addEventListener("click", createCompany);
document.getElementById("company_update").addEventListener("click", updateCompany);
document.getElementById("company_invite_signatory").addEventListener("click", inviteSignatory);
document.getElementById("company_remove_signatory").addEventListener("click", removeSignatory);
document.getElementById("company_list").addEventListener("click", listCompanies);
document.getElementById("dev_mode_get_company_chain").addEventListener("click", devModeGetCompanyChain);
document.getElementById("list_signatories").addEventListener("click", listSignatories);
document.getElementById("sync_company_chain").addEventListener("click", syncCompanyChain);
document.getElementById("company_detail").addEventListener("click", companyDetail);
document.getElementById("company_create_id").addEventListener("click", companyCreateId);
document.getElementById("confirm_company_email").addEventListener("click", confirmCompanyEmail);
document.getElementById("verify_company_email").addEventListener("click", verifyCompanyEmail);
document.getElementById("get_company_confirmations").addEventListener("click", getCompanyConfirmations);
document.getElementById("change_signatory_email").addEventListener("click", changeSignatoryEmail);
document.getElementById("get_company_invites").addEventListener("click", getCompanyInvites);
document.getElementById("company_accept_invite").addEventListener("click", acceptCompanyInvite);
document.getElementById("company_reject_invite").addEventListener("click", rejectCompanyInvite);
document.getElementById("locally_hide_signatory").addEventListener("click", locallyHideRemovedSignatory);
document.getElementById("get_proof_file").addEventListener("click", fetchCompanyFile);

// restore account, backup seed phrase
document.getElementById("get_seed_phrase").addEventListener("click", getSeedPhrase);
document.getElementById("restore_account").addEventListener("click", restoreFromSeedPhrase);


let config = {
  log_level: "debug",
  // bitcoin_network: "regtest", // local reg test
  // esplora_base_url: "http://localhost:8094", // local reg test via docker-compose
  bitcoin_network: "testnet",
  esplora_base_url: "https://esplora.minibill.tech",
  // nostr_relays: ["ws://localhost:8080"],
  nostr_relays: ["wss://relay.wildcat0.clowder-dev.minibill.tech", "wss://relay.wildcat1.clowder-dev.minibill.tech"],
  // this would be the default with current relay config
  blossom_servers: ["https://relay.wildcat0.clowder-dev.minibill.tech", "https://relay.wildcat1.clowder-dev.minibill.tech"],
  // if set to true we will drop DMs from nostr that we don't have in contacts
  nostr_only_known_contacts: false,
  job_runner_initial_delay_seconds: 5,
  job_runner_check_interval_seconds: 60,
  transport_initial_subscription_delay_seconds: 1,
  default_mint_url: "https://mint.wildcat0.clowder-dev.minibill.tech",
  default_mint_node_id: "bitcrt020e50d48b6b2897743ca257c82684e984509c05c9bf812176c717005698e57023", //wildcat0 clowder-dev
  num_confirmations_for_payment: 1,
  dev_mode: true,
  mandatory_email_confirmations: false,
  // default_court_url: "http://localhost:8000",
  default_court_url: "https://bcr-court-dev.minibill.tech"
};

document.getElementById("blossom_base_url").value = config.blossom_servers?.[0] || config.nostr_relays?.[0] || "";


async function start(create_identity) {
  await wasm.default();
  await wasm.initialize_api(config);

  window.notifApi = wasm.Api.notification();
  window.identityApi = wasm.Api.identity();
  window.contactApi = wasm.Api.contact();
  window.companyApi = wasm.Api.company();
  window.billApi = wasm.Api.bill();
  window.generalApi = wasm.Api.general();

  console.log("Apis initialized..");

  // Identity
  let identity;
  try {
    identity = success_or_fail(await window.identityApi.detail());
    console.log("local identity:", identity);
  } catch (err) {
    if (create_identity) {
      await sleep(2000); // sleep to let Nostr connect before first setup
      console.log("No local identity found - creating anon identity..");
      fail_on_error(await window.identityApi.create({
        t: 1,
        name: "Cypherpunk",
        postal_address: {},
      }));

      identity = success_or_fail(await window.identityApi.detail());

      console.log("Deanonymizing identity..");
      fail_on_error(await window.identityApi.deanonymize({
        t: 0,
        name: "Johanna Smith",
        email: "jsmith@example.com",
        postal_address: {
          country: "AT",
          city: "Vienna",
          zip: "1020",
          address: "street 1",
        }
      }));
    }
  }

  let current_identity;
  if (identity) {
    document.getElementById("identity").innerHTML = identity.node_id;

    await window.notifApi.subscribe((evt) => {
      console.log("Received event in JS: ", evt);
    });

    current_identity = success_or_fail(await window.identityApi.active());
    console.log(current_identity);
    document.getElementById("current_identity").innerHTML = current_identity.node_id;

    let companies = success_or_fail(await window.companyApi.list());
    if (companies.companies.length > 0) {
      document.getElementById("companies").innerHTML = "node_id: " + companies.companies[0].id;
    }
  }

  // General
  let currencies = success_or_fail(await window.generalApi.currencies());
  console.log("currencies: ", currencies);

  let status = success_or_fail(await window.generalApi.status());
  console.log("status: ", status);

  if (identity) {
    let search = success_or_fail(await window.generalApi.search({ filter: { search_term: "Test", currency: "SAT", item_types: ["Contact"] } }));
    console.log("search: ", search);
  }

  // Notifications
  if (current_identity) {
    let filter = { node_ids: [current_identity.node_id] };
    let notifications = success_or_fail(await window.notifApi.list(filter));
    console.log("notifications: ", notifications);
  }
  console.log("Returning apis..");
}

await start(generateIdentity());

async function uploadFile(event) {
  const file = event.target.files[0];
  if (!file) return;

  const name = file.name;
  const extension = name.split('.').pop();

  const bytes = await file.arrayBuffer();
  const data = new Uint8Array(bytes);

  const uploadedFile = { name, extension, data };

  console.log("File Name:", uploadedFile.name);
  console.log("File Extension:", uploadedFile.extension);
  console.log("File Bytes:", uploadedFile.data);
  try {
    let file_upload_response = success_or_fail(await window.contactApi.upload(uploadedFile));
    console.log("success uploading:", file_upload_response);
    document.getElementById("file_upload_id").value = file_upload_response.file_upload_id;
  } catch (err) {
    console.log("upload error: ", err);
  }
}

async function attachIdentityProfilePicture() {
  const file_upload_id = document.getElementById("file_upload_id").value;
  if (!file_upload_id) {
    console.log("No file_upload_id set.");
    return;
  }

  fail_on_error(await window.identityApi.change({
    profile_picture_file_upload_id: file_upload_id,
    postal_address: {}
  }));

  const identity = success_or_fail(await window.identityApi.detail());
  document.getElementById("node_id_identity").value = identity.node_id;
  if (identity.profile_picture_file) {
    document.getElementById("blossom_nostr_hash").value = identity.profile_picture_file.nostr_hash;
    buildBlossomUrl();
  }
  console.log("attached uploaded file as identity profile picture:", identity);
}

async function attachIdentityDocument() {
  const file_upload_id = document.getElementById("file_upload_id").value;
  if (!file_upload_id) {
    console.log("No file_upload_id set.");
    return;
  }

  fail_on_error(await window.identityApi.change({
    identity_document_file_upload_id: file_upload_id,
    postal_address: {}
  }));

  const identity = success_or_fail(await window.identityApi.detail());
  document.getElementById("node_id_identity").value = identity.node_id;
  if (identity.identity_document_file) {
    document.getElementById("blossom_nostr_hash").value = identity.identity_document_file.nostr_hash;
    buildBlossomUrl();
  }
  console.log("attached uploaded file as identity document:", identity);
}

async function attachContactAvatar() {
  const file_upload_id = document.getElementById("file_upload_id").value;
  const node_id = document.getElementById("contact_id").value || document.getElementById("node_id_contact").value;
  if (!file_upload_id || !node_id) {
    console.log("Need both file_upload_id and contact node id.");
    return;
  }

  fail_on_error(await window.contactApi.edit({
    node_id,
    avatar_file_upload_id: file_upload_id,
    postal_address: {}
  }));

  const contact = success_or_fail(await window.contactApi.detail(node_id));
  console.log("attached uploaded file as contact avatar:", contact);
  document.getElementById("contact_id").value = contact.node_id;
  document.getElementById("node_id_contact").value = contact.node_id;
  if (contact.avatar_file) {
    document.getElementById("contact_file_name").value = contact.avatar_file.name;
    document.getElementById("blossom_nostr_hash").value = contact.avatar_file.nostr_hash;
    buildBlossomUrl();
  }
}

async function attachCompanyProof() {
  const file_upload_id = document.getElementById("file_upload_id").value;
  const id = document.getElementById("company_id").value;
  if (!file_upload_id || !id) {
    console.log("Need both file_upload_id and company id.");
    return;
  }

  fail_on_error(await window.companyApi.edit({
    id,
    proof_of_registration_file_upload_id: file_upload_id,
    postal_address: {}
  }));

  const company = success_or_fail(await window.companyApi.detail(id));
  document.getElementById("company_id").value = company.id;
  if (company.proof_of_registration_file) {
    document.getElementById("blossom_nostr_hash").value = company.proof_of_registration_file.nostr_hash;
    buildBlossomUrl();
  }
  console.log("attached uploaded file as company proof:", company);
}

async function removeCompanyProof() {
  const id = document.getElementById("company_id").value;
  fail_on_error(await window.companyApi.edit({
    id,
    proof_of_registration_file_upload_id: undefined,
    postal_address: {}
  }));

  const company = success_or_fail(await window.companyApi.detail(id));
  console.log("removed uploaded file as company proof:", company);
}

function buildBlossomUrl() {
  const baseUrl = document.getElementById("blossom_base_url").value?.trim();
  const nostrHash = document.getElementById("blossom_nostr_hash").value?.trim();
  const target = document.getElementById("blossom_direct_url");

  if (!baseUrl || !nostrHash) {
    target.href = "#";
    target.textContent = "";
    return null;
  }

  const url = new URL(nostrHash, baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`);
  target.href = url.toString();
  target.textContent = url.toString();
  return url.toString();
}

function openBlossomUrl() {
  const url = buildBlossomUrl();
  if (!url) {
    console.log("Need both Blossom base URL and nostr hash.");
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

async function getSeedPhrase() {
  let seed_phrase = success_or_fail(await window.identityApi.seed_backup());
  document.getElementById("current_seed").innerHTML = seed_phrase.seed_phrase;
}

async function restoreFromSeedPhrase() {
  let seed_phrase = document.getElementById("restore_seed_phrase").value;
  fail_on_error(await window.identityApi.seed_recover({ seed_phrase }));
}

async function createCompany() {
  let company_id = document.getElementById("company_id").value;
  let company_email = document.getElementById("company_email").value;
  let company = success_or_fail(await window.companyApi.create({
    id: company_id,
    name: "hayek Ltd",
    email: "test@example.com",
    postal_address: {
      country: "AT",
      city: "Vienna",
      zip: "1020",
      address: "street 1",
    },
    creator_email: company_email,
    proof_of_registration_file_upload_id: document.getElementById("file_upload_id").value || undefined,
  }));
  console.log("company: ", company);
}

async function updateCompany() {
  let company_id = document.getElementById("company_id").value;
  let name = document.getElementById("company_update_name").value;
  fail_on_error(await window.companyApi.edit({
    id: company_id,
    name: name,
    postal_address: {}
  }));
  console.log("updated company name: ", company_id, name);
}

async function inviteSignatory() {
  let company_id = document.getElementById("company_id").value;
  let signatory_node_id = document.getElementById("company_signatory_id").value;
  fail_on_error(await window.companyApi.invite_signatory({
    id: company_id,
    signatory_node_id: signatory_node_id,
  }));
  console.log("invited signatory to company: ", signatory_node_id, company_id);
}

async function removeSignatory() {
  let company_id = document.getElementById("company_id").value;
  let signatory_node_id = document.getElementById("company_signatory_id").value;
  fail_on_error(await window.companyApi.remove_signatory({
    id: company_id,
    signatory_node_id: signatory_node_id,
  }));
  console.log("removed signatory to company: ", signatory_node_id, company_id);
}

async function listCompanies() {
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.list());
  });
  await measured();
}

async function listSignatories() {
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.list_signatories(document.getElementById("company_id").value));
  });
  await measured();
}

async function triggerContact() {
  let node_id = document.getElementById("node_id_contact").value;
  try {
    let contact = success_or_fail(await window.contactApi.detail(node_id));
    console.log("contact:", contact);
  } catch (err) {
    console.log("No contact found - creating..");
    let file_upload_id = document.getElementById("file_upload_id").value || undefined;
    fail_on_error(await window.contactApi.create({
      t: 0,
      node_id: node_id,
      name: "Test Contact",
      email: "text@example.com",
      postal_address: {
        country: "AT",
        city: "Vienna",
        zip: "1020",
        address: "street 1",
      },
      avatar_file_upload_id: file_upload_id,
    }));
  }
  let contact = success_or_fail(await window.contactApi.detail(node_id));
  console.log("contact:", contact);
  document.getElementById("contact_id").value = node_id;
  document.getElementById("node_id_bill").value = node_id;
  if (contact.avatar_file) {
    document.getElementById("contact_file_name").value = contact.avatar_file.name;
  }
}

async function triggerAnonContact() {
  let node_id = document.getElementById("node_id_contact").value;
  try {
    let contact = success_or_fail(await window.contactApi.detail(node_id));
    console.log("anon contact:", contact);
  } catch (err) {
    console.log("No contact found - creating..");
    fail_on_error(await window.contactApi.create({
      t: 2,
      node_id: node_id,
      name: "some anon dude",
      email: "text@example.com",
    }));
  }
  document.getElementById("contact_id").value = node_id;
  document.getElementById("node_id_bill").value = node_id;
}

async function triggerBill(t, blank) {
  let measured = measure(async () => {
    console.log("creating bill");

    const now = new Date();
    const issue_date = now.toISOString().split('T')[0];
    const nMonthsLater = new Date(now); // maturity date today end of day
    // nMonthsLater.setMonth(nMonthsLater.getMonth() + 1);
    const maturity_date = nMonthsLater.toISOString().split('T')[0];

    let file_upload_id = document.getElementById("file_upload_id").value || undefined;
    let node_id = document.getElementById("node_id_bill").value;
    let identity = success_or_fail(await window.identityApi.detail());
    let bill_issue_data = {
      t,
      country_of_issuing: "at",
      city_of_issuing: "Vienna",
      issue_date,
      maturity_date,
      payee: t == 0 ? node_id : identity.node_id,
      drawee: t == 0 ? identity.node_id : node_id,
      sum: "15000",
      currency: "SAT",
      country_of_payment: "GB",
      city_of_payment: "London",
      file_upload_ids: file_upload_id ? [file_upload_id] : []
    };
    let bill;
    if (blank) {
      bill = success_or_fail(await window.billApi.issue_blank(bill_issue_data));
    } else {
      bill = success_or_fail(await window.billApi.issue(bill_issue_data));
    }
    let bill_id = bill.id;
    console.log("created bill with id: ", bill_id);
  });
  await measured();
}

async function triggerNotif() {
  fail_on_error(await window.notifApi.trigger_test_msg({ test: "Hello, World" }));
}

async function fetchTempFile() {
  let file_upload_id = document.getElementById("file_upload_id").value;
  let temp_file = success_or_fail(await window.generalApi.temp_file(file_upload_id));
  let file_bytes = temp_file.data;
  let arr = new Uint8Array(file_bytes);
  let blob = new Blob([arr], { type: temp_file.content_type });
  let url = URL.createObjectURL(blob);

  console.log("file", temp_file, url, blob);
  document.getElementById("uploaded_file").src = url;
}

async function fetchContactFile() {
  let node_id = document.getElementById("contact_id").value;
  let file_name = document.getElementById("contact_file_name").value;
  let file = success_or_fail(await window.contactApi.file_base64(node_id, file_name));
  document.getElementById("attached_file").src = `data:${file.content_type};base64,${file.data}`;
}

async function switchIdentity() {
  let node_id = document.getElementById("node_id_identity").value;
  fail_on_error(await window.identityApi.switch({ t: 1, node_id }));
  document.getElementById("current_identity").textContent = node_id;
}

async function shareContact() {
  let node_id = document.getElementById("share_recipient_node_id").value;
  console.log("sharing contact details to identity: ", node_id);
  fail_on_error(await window.identityApi.share_contact_details({ recipient: node_id }));
}

async function shareCompanyContact() {
  let company_id = document.getElementById("share_company_id").value;
  let share_to_node_id = document.getElementById("share_company_recipient_node_id").value;
  console.log("sharing company contact details to identity: ", company_id, "->", share_to_node_id);
  fail_on_error(await window.companyApi.share_contact_details({ recipient: share_to_node_id, company_id: company_id }));
}

async function listPendingContactShares() {
  let receiver_node_id = document.getElementById("receiver_node_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.list_pending_contact_shares(receiver_node_id));
  });
  await measured();
}

async function getPendingContactShare() {
  let pending_share_id = document.getElementById("pending_share_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.get_pending_contact_share(pending_share_id));
  });
  await measured();
}

async function approveContactShare() {
  let pending_share_id = document.getElementById("pending_share_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.approve_contact_share({
      pending_share_id: pending_share_id,
      add_to_contacts: true,
      share_back: false
    }));
  });
  await measured();
}

async function approveContactShareWithShareBack() {
  let pending_share_id = document.getElementById("pending_share_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.approve_contact_share({
      pending_share_id: pending_share_id,
      add_to_contacts: true,
      share_back: true
    }));
  });
  await measured();
}

async function approveContactShareOnlyShareBack() {
  let pending_share_id = document.getElementById("pending_share_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.approve_contact_share({
      pending_share_id: pending_share_id,
      add_to_contacts: false,
      share_back: true
    }));
  });
  await measured();
}

async function rejectContactShare() {
  let pending_share_id = document.getElementById("pending_share_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.reject_contact_share(pending_share_id));
  });
  await measured();
}

async function endorseBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let endorsee = document.getElementById("endorsee_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.endorse_bill({ bill_id, endorsee }));
  });
  await measured();
}

async function endorseBillBlank() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let endorsee = document.getElementById("endorsee_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.endorse_bill_blank({ bill_id, endorsee }));
  });
  await measured();
}

async function requestToAcceptBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.request_to_accept({ bill_id, acceptance_deadline: getDeadlineDate() }));
  });
  await measured();
}

async function acceptBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.accept({ bill_id }));
  });
  await measured();
}

async function requestToPayBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.request_to_pay({ bill_id, currency: "SAT", payment_deadline: getDeadlineDate() }));
  });
  await measured();
}

async function offerToSellBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let endorsee = document.getElementById("endorsee_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.offer_to_sell({ bill_id, sum: "500", currency: "SAT", buyer: endorsee, buying_deadline: getDeadlineDate() }));
  });
  await measured();
}

async function requestToRecourseBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let endorsee = document.getElementById("endorsee_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.request_to_recourse_bill_acceptance({ bill_id, recoursee: endorsee, recourse_deadline: getDeadlineDate() }));
  });
  await measured();
}

async function requestToRecourseBillPayment() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let endorsee = document.getElementById("endorsee_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.request_to_recourse_bill_payment({ bill_id, recoursee: endorsee, currency: "SAT", sum: "1500", recourse_deadline: getDeadlineDate() }));
  });
  await measured();
}

async function rejectAcceptBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.reject_to_accept({ bill_id }));
  });
  await measured();
}

async function rejectPayBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.reject_to_pay({ bill_id }));
  });
  await measured();
}

async function rejectBuyingBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.reject_to_buy({ bill_id }));
  });
  await measured();
}

async function rejectRecourseBill() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.reject_to_pay_recourse({ bill_id }));
  });
  await measured();
}

async function requestToMint() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.request_to_mint({ bill_id, mint_node: config.default_mint_node_id }));
  });
  await measured();
}

async function getMintState() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.mint_state(bill_id));
  });
  await measured();
}

async function checkMintState() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.check_mint_state(bill_id));
  });
  await measured();
}

async function cancelRegToMint() {
  let mint_request_id = document.getElementById("mint_req_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.cancel_request_to_mint(mint_request_id));
  });
  await measured();
}

async function acceptMintOffer() {
  let mint_request_id = document.getElementById("mint_req_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.accept_mint_offer(mint_request_id));
  });
  await measured();
}

async function rejectMintOffer() {
  let mint_request_id = document.getElementById("mint_req_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.reject_mint_offer(mint_request_id));
  });
  await measured();
}

async function fetchBillDetail() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.detail(document.getElementById("bill_id").value));
  });
  await measured();
}

async function fetchBillEndorsements() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.endorsements(document.getElementById("bill_id").value));
  });
  await measured();
}

async function fetchBillPastEndorsees() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.past_endorsees(document.getElementById("bill_id").value));
  });
  await measured();
}

async function fetchBillPastPayments() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.past_payments(document.getElementById("bill_id").value));
  });
  await measured();
}

async function fetchBillFile() {
  let bill_id = document.getElementById("bill_id").value;
  let detail = success_or_fail(await window.billApi.detail(bill_id));

  if (detail.data.files.length > 0) {
    let file = success_or_fail(await window.billApi.attachment_base64(bill_id, detail.data.files[0].name));
    document.getElementById("bill_attached_file").src = `data:${file.content_type};base64,${file.data}`;
  } else {
    console.log("Bill has no file");
  }
}

async function fetchBillBills() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.list());
  });
  await measured();
}

async function fetchBillBalances() {
  let measured = measure(async () => {
    return success_or_fail(await window.generalApi.overview("SAT"));
  });
  await measured();
}

async function fetchBillSearch() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.search({ filter: { currency: "SAT", role: "All", participants: [] } }));
  });
  await measured();
}

async function fetchBillHistory() {
  let bill_id = document.getElementById("bill_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.bill_history(bill_id));
  });
  await measured();
}

async function clearBillCache() {
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.clear_bill_cache());
  });
  await measured();
}

async function getBitcoinKeys() {
  let bill_id = document.getElementById("bill_id").value;
  console.log("getBitcoinKeys", bill_id);
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.bitcoin_keys(bill_id));
  });
  await measured();
}

async function syncBillChain() {
  let bill_id = document.getElementById("bill_id").value;
  console.log("syncBillChain", bill_id);
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.sync_bill_chain({ bill_id: bill_id, from_nostr: false }));
  });
  await measured();
}

async function syncBillChainFromNostr() {
  let bill_id = document.getElementById("bill_id").value;
  console.log("syncBillChain from Nostr", bill_id);
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.sync_bill_chain({ bill_id: bill_id, from_nostr: true }));
  });
  await measured();
}

async function syncCompanyChain() {
  let node_id = document.getElementById("company_id").value;
  console.log("syncCompanyChain", node_id);
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.sync_company_chain({ node_id: node_id }));
  });
  await measured();
}

async function fetchCompanyFile() {
  let node_id = document.getElementById("company_id").value;
  let detail = success_or_fail(await window.companyApi.detail(node_id));
  let file_name = detail.proof_of_registration_file.name;
  let file = success_or_fail(await window.companyApi.file_base64(node_id, file_name));

  document.getElementById("bill_attached_file").src = `data:${file.content_type};base64,${file.data}`;
}

async function companyDetail() {
  let node_id = document.getElementById("company_id").value;
  console.log("companyDetail", node_id);
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.detail(node_id));
  });
  await measured();
}

async function companyCreateId() {
  console.log("companyCreateId");
  let id = success_or_fail(await window.companyApi.create_keys());
  console.log(id);
  document.getElementById("company_id").value = id.id;
}

async function confirmCompanyEmail() {
  console.log("confirmCompanyEmail");
  let email = document.getElementById("company_email").value;
  let id = document.getElementById("company_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.confirm_email({ id, email }));
  });
  await measured();
}

async function verifyCompanyEmail() {
  console.log("verifyCompanyEmail");
  let code = document.getElementById("company_confirmation_code").value;
  let id = document.getElementById("company_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.verify_email({ id, confirmation_code: code }));
  });
  await measured();
}

async function getCompanyConfirmations() {
  console.log("getCompanyConfirmations");
  let id = document.getElementById("company_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.get_email_confirmations(id));
  });
  await measured();
}

async function changeSignatoryEmail() {
  console.log("changeSignatoryEmail");
  let id = document.getElementById("company_id").value;
  let email = document.getElementById("company_email").value;
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.change_signatory_email({ id, email }));
  });
  await measured();
}

async function getCompanyInvites() {
  console.log("getCompanyInvites");
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.list_invites());
  });
  await measured();
}

async function acceptCompanyInvite() {
  let id = document.getElementById("company_id").value;
  let email = document.getElementById("company_email").value;
  console.log("acceptCompanyInvite");
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.accept_invite({ id, email }));
  });
  await measured();
}

async function rejectCompanyInvite() {
  let id = document.getElementById("company_id").value;
  console.log("rejectCompanyInvite");
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.reject_invite(id));
  });
  await measured();
}
async function locallyHideRemovedSignatory() {
  let id = document.getElementById("company_id").value;
  let signatory_node_id = document.getElementById("company_signatory_id").value;
  console.log("locallyHideRemovedSignatory");
  let measured = measure(async () => {
    return success_or_fail(await window.companyApi.locally_hide_signatory({ id, signatory_node_id }));
  });
  await measured();
}


async function syncIdentityChain() {
  console.log("syncIdentityChain");
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.sync_identity_chain());
  });
  await measured();
}

async function confirmEmail() {
  console.log("confirmEmail");
  let email = document.getElementById("identity_email").value;
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.confirm_email({ email }));
  });
  await measured();
}

async function verifyEmail() {
  console.log("verifyEmail");
  let code = document.getElementById("confirmation_code").value;
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.verify_email({ confirmation_code: code }));
  });
  await measured();
}

async function changeEmail() {
  console.log("changeEmail");
  let email = document.getElementById("identity_email").value;
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.change_email({ email }));
  });
  await measured();
}

async function editIdentity() {
  console.log("editIdentity");
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.change({ postal_address: { country: "AT", zip: "1030" } }));
  });
  await measured();
}

async function getIdentityConfirmations() {
  console.log("getIdentityConfirmations");
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.get_email_confirmations());
  });
  await measured();
}

async function getIdentity() {
  console.log("getIdentity");
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.detail());
  });
  await measured();
}

async function changeName() {
  console.log("changeName");
  let name = document.getElementById("identity_name").value;
  let measured = measure(async () => {
    return success_or_fail(await window.identityApi.change({ name, postal_address: {} }));
  });
  await measured();
}

async function shareBillWithCourt() {
  let bill_id = document.getElementById("court_bill_id").value;
  let court_node_id = document.getElementById("court_node_id").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.share_bill_with_court({ bill_id: bill_id, court_node_id: court_node_id }));
  });
  await measured();
}

async function mempoolLink() {
  let btc_address = document.getElementById("btc_address").value;
  let measured = measure(async () => {
    return success_or_fail(await window.generalApi.mempool_link({ address: btc_address }));
  });
  await measured();
}

async function linkToPay() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let btc_address = document.getElementById("btc_address").value;
  let sum = document.getElementById("payment_sum").value;
  let measured = measure(async () => {
    return success_or_fail(await window.generalApi.link_to_pay({ bill_id: bill_id, address: btc_address, sum: sum }));
  });
  await measured();
}

async function checkAndEstimateSweep() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let btc_address = document.getElementById("btc_address").value;
  let dest_btc_address = document.getElementById("destination_btc_address").value;
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.check_and_estimate_btc_sweep({ bill_id: bill_id, source_address: btc_address, destination_address: dest_btc_address }));
  });
  await measured();
}

async function sweepFunds() {
  let bill_id = document.getElementById("endorse_bill_id").value;
  let btc_address = document.getElementById("btc_address").value;
  let dest_btc_address = document.getElementById("destination_btc_address").value;
  let fee = Number.parseInt(document.getElementById("sweep_fee").value);
  let measured = measure(async () => {
    return success_or_fail(await window.billApi.sweep_btc_funds({ bill_id: bill_id, source_address: btc_address, destination_address: dest_btc_address, fee }));
  });
  await measured();
}

async function devModeGetBillChain() {
  let bill_id = document.getElementById("dev_mode_bill_id").value;
  console.log("devModeGetBillChain", bill_id);
  let measured = measure(async () => {
    let res = success_or_fail(await window.billApi.dev_mode_get_full_bill_chain(bill_id));
    return res.map((b) => {
      return JSON.parse(b);
    })
  });
  await measured();
}

async function devModeGetIdentityChain() {
  console.log("devModeGetIdentityChain");
  let measured = measure(async () => {
    let res = success_or_fail(await window.identityApi.dev_mode_get_full_identity_chain());
    return res.map((b) => {
      return JSON.parse(b);
    })
  });
  await measured();
}

async function devModeGetCompanyChain() {
  let company_id = document.getElementById("dev_mode_company_id").value;
  console.log("devModeGetCompanyChain", company_id);
  let measured = measure(async () => {
    let res = success_or_fail(await window.companyApi.dev_mode_get_full_company_chain(company_id));
    return res.map((b) => {
      return JSON.parse(b);
    })
  });
  await measured();
}

function measure(promiseFunction) {
  return async function(...args) {
    const startTime = performance.now();
    const result = await promiseFunction(...args);
    const endTime = performance.now();
    const exec_time = (endTime - startTime).toFixed(2);
    console.log(`Execution time: ${exec_time} ms`);

    document.getElementById("bill_execution_time").innerHTML = `${exec_time} ms`;
    document.getElementById("bill_results").innerHTML = "<pre>" + JSON.stringify(result, null, 2) + "</pre>";

    return result;
  };
}

async function fetchContacts() {
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.list());
  });
  await measured();
}

async function searchContacts() {
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.search({
      search_term: document.getElementById("contact_search_term").value,
      include_logical: true,
      include_contact: true
    }));
  });
  await measured();
}

async function removeContactAvatar() {
  let node_id = document.getElementById("node_id_contact").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.edit({ node_id: node_id, avatar_file_upload_id: undefined, postal_address: {} }));
  });
  await measured();
}

async function editContact() {
  let node_id = document.getElementById("node_id_contact").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.edit({ node_id: node_id, postal_address: { country: "AT", zip: undefined } }));
  });
  await measured();
}

async function deleteContact() {
  let node_id = document.getElementById("node_id_contact").value;
  let measured = measure(async () => {
    return success_or_fail(await window.contactApi.remove(node_id));
  });
  await measured();
}

async function getActiveNotif() {
  let measured = measure(async () => {
    return success_or_fail(await window.notifApi.active_notifications_for_node_ids([]));
  });
  await measured();
}

async function getNotifList() {
  let measured = measure(async () => {
    return success_or_fail(await window.notifApi.list({}));
  });
  await measured();
}

async function get_email_notifications_preferences_link() {
  let measured = measure(async () => {
    return success_or_fail(await window.notifApi.get_email_notifications_preferences_link());
  });
  await measured();
}

// disables auto identity creation via query param identity=false
function generateIdentity() {
  let param = getQueryParam("identity");
  if (param && param.toLowerCase() === "false") {
    return false;
  }
  return true;
}

function getQueryParam(paramName) {
  const urlParams = new URLSearchParams(window.location.search);
  return urlParams.get(paramName);
}

function getDeadlineDate() {
  const now = new Date();
  const nDaysLater = new Date(now);
  nDaysLater.setDate(now.getDate() + 3); // set deadline to 3 days later
  return nDaysLater.toISOString().split('T')[0]
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// Use to extract the data from a TSResult::Success, or throw if it's an error
function success_or_fail(res) {
  if (res.Error) {
    throw (res.Error);
  } else {
    return res.Success;
  }
}

// Use to throw an exception on error, and otherwise just return the result
function fail_on_error(res) {
  if (res.Error) {
    throw (res.Error);
  }
}
