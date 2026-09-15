use std::str::FromStr;

use super::{BillIdDb, Result, surreal::Bindings};
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_ebill_core::{
    application::ServiceTraitBounds,
    protocol::Sum,
    protocol::Timestamp,
    protocol::mint::{MintOffer, MintOfferRecoveryData, MintRequest, MintRequestStatus},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Error,
    constants::{
        DB_BILL_ID, DB_MINT_NODE_ID, DB_MINT_REQUEST_ID, DB_MINT_REQUESTER_NODE_ID, DB_PROOFS,
        DB_PROOFS_SPENT, DB_RECOVERY_DATA, DB_STATUS, DB_STATUS_ACCEPTED, DB_STATUS_MINTINGENABLED,
        DB_STATUS_OFFERED, DB_STATUS_PENDING, DB_TABLE,
    },
    mint::MintStoreApi,
};

use super::surreal::SurrealWrapper;

#[derive(Clone)]
pub struct SurrealMintStore {
    db: SurrealWrapper,
}

impl SurrealMintStore {
    const REQUESTS_TABLE: &'static str = "requests";
    const OFFERS_TABLE: &'static str = "offers";

    pub fn new(db: SurrealWrapper) -> Self {
        Self { db }
    }
}

impl ServiceTraitBounds for SurrealMintStore {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl MintStoreApi for SurrealMintStore {
    async fn exists_for_bill(&self, requester_node_id: &NodeId, bill_id: &BillId) -> Result<bool> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_BILL_ID, bill_id.to_owned())?;
        bindings.add(DB_MINT_REQUESTER_NODE_ID, requester_node_id.to_owned())?;
        match self
            .db
            .query::<Option<BillIdDb>>(
                "SELECT bill_id FROM type::table($table) WHERE bill_id = $bill_id AND requester_node_id = $requester_node_id GROUP BY bill_id",
                bindings,
            )
            .await
        {
            Ok(res) => {
                if res.is_empty() {
                    // not found
                    Ok(false)
                } else {
                    // found - check keys
                    Ok(true)
                }
            }
            Err(e) => {
                log::error!(
                    "Error checking if there are mint requests for bill {bill_id} exists: {e}"
                );
                Ok(false)
            }
        }
    }

    async fn dev_mode_reset_for_bill(&self, bill_id: &BillId) -> Result<()> {
        let mut bindings = Bindings::default();
        bindings.add("requests_table", Self::REQUESTS_TABLE)?;
        bindings.add("offers_table", Self::OFFERS_TABLE)?;
        bindings.add(DB_BILL_ID, bill_id.to_owned())?;

        // Offers only know the mint_request_id, so remove offers belonging
        // to requests for this bill first.
        self.db
            .query_check(
                "DELETE FROM type::table($offers_table) WHERE mint_request_id IN (SELECT VALUE mint_request_id FROM type::table($requests_table) WHERE bill_id = $bill_id)",
                bindings.clone(),
            )
            .await?;

        self.db
            .query_check(
                "DELETE FROM type::table($requests_table) WHERE bill_id = $bill_id",
                bindings,
            )
            .await?;

        Ok(())
    }

    async fn get_all_active_requests(&self) -> Result<Vec<MintRequest>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_STATUS_OFFERED, MintRequestStatusDb::Offered)?;
        bindings.add(DB_STATUS_PENDING, MintRequestStatusDb::Pending)?;
        bindings.add(DB_STATUS_ACCEPTED, MintRequestStatusDb::Accepted)?;
        bindings.add(
            DB_STATUS_MINTINGENABLED,
            MintRequestStatusDb::MintingEnabled,
        )?;
        let results: Vec<MintRequestDb> = self.db
            .query("SELECT * from type::table($table) WHERE status = $status_offered OR status = $status_pending OR status = $status_accepted OR status = $status_mintingenabled", bindings).await?;
        results.into_iter().map(|c| c.try_into()).collect()
    }

    async fn get_requests(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
        mint_node_id: &NodeId,
    ) -> Result<Vec<MintRequest>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_BILL_ID, bill_id.to_owned())?;
        bindings.add(DB_MINT_NODE_ID, mint_node_id.to_owned())?;
        bindings.add(DB_MINT_REQUESTER_NODE_ID, requester_node_id.to_owned())?;
        let results: Vec<MintRequestDb> = self.db
            .query("SELECT * from type::table($table) WHERE bill_id = $bill_id AND mint_node_id = $mint_node_id AND requester_node_id = $requester_node_id", bindings).await?;
        results.into_iter().map(|c| c.try_into()).collect()
    }

    async fn get_requests_for_bill(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
    ) -> Result<Vec<MintRequest>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_BILL_ID, bill_id.to_owned())?;
        bindings.add(DB_MINT_REQUESTER_NODE_ID, requester_node_id.to_owned())?;
        let results: Vec<MintRequestDb> = self.db
            .query("SELECT * from type::table($table) WHERE bill_id = $bill_id AND requester_node_id = $requester_node_id", bindings).await?;
        results.into_iter().map(|c| c.try_into()).collect()
    }

    async fn add_request(
        &self,
        requester_node_id: &NodeId,
        bill_id: &BillId,
        mint_node_id: &NodeId,
        mint_request_id: &Uuid,
        timestamp: Timestamp,
    ) -> Result<()> {
        let entity = MintRequestDb {
            requester_node_id: requester_node_id.to_owned(),
            bill_id: bill_id.to_owned(),
            mint_node_id: mint_node_id.to_owned(),
            mint_request_id: mint_request_id.to_string(),
            timestamp,
            status: MintRequestStatusDb::Pending,
        };
        let _: Option<MintRequestDb> = self.db.create(Self::REQUESTS_TABLE, None, entity).await?;
        Ok(())
    }

    async fn get_request(&self, mint_request_id: &Uuid) -> Result<Option<MintRequest>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        let results: Vec<MintRequestDb> = self
            .db
            .query(
                "SELECT * from type::table($table) WHERE mint_request_id = $mint_request_id",
                bindings,
            )
            .await?;
        results.first().map(|r| r.clone().try_into()).transpose()
    }

    async fn update_request(
        &self,
        mint_request_id: &Uuid,
        new_status: &MintRequestStatus,
    ) -> Result<()> {
        let mut bindings = Bindings::default();
        let status: MintRequestStatusDb = new_status.clone().into();
        bindings.add(DB_TABLE, Self::REQUESTS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        bindings.add(DB_STATUS, status)?;
        self.db
            .query_check("UPDATE type::table($table) SET status = $status WHERE mint_request_id = $mint_request_id", bindings)
            .await?;
        Ok(())
    }

    async fn add_proofs_to_offer(&self, mint_request_id: &Uuid, proofs: &str) -> Result<()> {
        // we only add proofs, if there is an offer and it has no proofs yet
        if let Ok(Some(offer)) = self.get_offer(mint_request_id).await {
            if offer.proofs.is_some() {
                return Err(Error::Conflict("mint offer already has proofs".to_string()));
            }
        } else {
            return Err(Error::NoSuchEntity(
                "mint offer".to_string(),
                mint_request_id.to_string(),
            ));
        }
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::OFFERS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        bindings.add(DB_PROOFS, Some(proofs.to_owned()))?;
        self.db
            .query_check("UPDATE type::table($table) SET proofs = $proofs WHERE mint_request_id = $mint_request_id", bindings)
            .await?;
        Ok(())
    }

    async fn add_recovery_data_to_offer(
        &self,
        mint_request_id: &Uuid,
        secrets: &[String],
        rs: &[String],
    ) -> Result<()> {
        // we only add recovery data, if there is an offer and it has no proofs and no recovery data yet
        if let Ok(Some(offer)) = self.get_offer(mint_request_id).await {
            if offer.proofs.is_some() {
                return Err(Error::Conflict("mint offer already has proofs".to_string()));
            }

            if offer.recovery_data.is_some() {
                return Err(Error::Conflict(
                    "mint offer already has recovery data".to_string(),
                ));
            }
        } else {
            return Err(Error::NoSuchEntity(
                "mint offer".to_string(),
                mint_request_id.to_string(),
            ));
        }
        let recovery_data = MintOfferRecoveryDataDb {
            secrets: secrets.to_owned(),
            rs: rs.to_owned(),
        };
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::OFFERS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        bindings.add(DB_RECOVERY_DATA, Some(recovery_data))?;
        self.db
            .query_check("UPDATE type::table($table) SET recovery_data = $recovery_data WHERE mint_request_id = $mint_request_id", bindings)
            .await?;
        Ok(())
    }

    async fn set_proofs_to_spent_for_offer(&self, mint_request_id: &Uuid) -> Result<()> {
        // we only set to spent, if there is an offer and it has proofs
        if let Ok(Some(offer)) = self.get_offer(mint_request_id).await {
            if offer.proofs.is_none() {
                return Err(Error::NoSuchEntity(
                    "offer proofs".to_string(),
                    mint_request_id.to_string(),
                ));
            }
        } else {
            return Err(Error::NoSuchEntity(
                "mint offer".to_string(),
                mint_request_id.to_string(),
            ));
        }
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::OFFERS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        bindings.add(DB_PROOFS_SPENT, true)?;
        self.db
            .query_check("UPDATE type::table($table) SET proofs_spent = $proofs_spent WHERE mint_request_id = $mint_request_id", bindings)
            .await?;
        Ok(())
    }

    async fn add_offer(
        &self,
        mint_request_id: &Uuid,
        keyset_id: &str,
        expiration_timestamp: Timestamp,
        discounted_sum: Sum,
    ) -> Result<()> {
        // we only add an offer, if there isn't already an offer for this request
        if let Ok(Some(_)) = self.get_offer(mint_request_id).await {
            return Err(Error::Conflict("mint offer already exists".to_string()));
        }
        let entity = MintOfferDb {
            mint_request_id: mint_request_id.to_string(),
            keyset_id: keyset_id.to_owned(),
            expiration_timestamp,
            discounted_sum,
            proofs: None,
            proofs_spent: false,
            recovery_data: None,
        };
        let _: Option<MintOfferDb> = self.db.create(Self::OFFERS_TABLE, None, entity).await?;
        Ok(())
    }

    async fn get_offer(&self, mint_request_id: &Uuid) -> Result<Option<MintOffer>> {
        let mut bindings = Bindings::default();
        bindings.add(DB_TABLE, Self::OFFERS_TABLE)?;
        bindings.add(DB_MINT_REQUEST_ID, mint_request_id.to_string())?;
        let results: Vec<MintOfferDb> = self
            .db
            .query(
                "SELECT * from type::table($table) WHERE mint_request_id = $mint_request_id",
                bindings,
            )
            .await?;
        results.first().map(|r| r.clone().try_into()).transpose()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintOfferDb {
    pub mint_request_id: String,
    pub keyset_id: String,
    pub expiration_timestamp: Timestamp,
    pub discounted_sum: Sum,
    pub proofs: Option<String>,
    pub proofs_spent: bool,
    pub recovery_data: Option<MintOfferRecoveryDataDb>,
}

impl TryFrom<MintOfferDb> for MintOffer {
    type Error = Error;

    fn try_from(value: MintOfferDb) -> Result<Self> {
        Ok(Self {
            mint_request_id: Uuid::from_str(&value.mint_request_id)
                .map_err(|_| Error::EncodingError)?,
            keyset_id: value.keyset_id,
            expiration_timestamp: value.expiration_timestamp,
            discounted_sum: value.discounted_sum,
            proofs: value.proofs,
            proofs_spent: value.proofs_spent,
            recovery_data: value.recovery_data.map(|rd| rd.into()),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintOfferRecoveryDataDb {
    pub secrets: Vec<String>,
    pub rs: Vec<String>,
}

impl From<MintOfferRecoveryDataDb> for MintOfferRecoveryData {
    fn from(value: MintOfferRecoveryDataDb) -> Self {
        Self {
            secrets: value.secrets,
            rs: value.rs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintRequestDb {
    pub requester_node_id: NodeId,
    pub bill_id: BillId,
    pub mint_node_id: NodeId,
    pub mint_request_id: String,
    pub timestamp: Timestamp,
    pub status: MintRequestStatusDb,
}

impl TryFrom<MintRequestDb> for MintRequest {
    type Error = Error;

    fn try_from(value: MintRequestDb) -> Result<Self> {
        Ok(Self {
            requester_node_id: value.requester_node_id,
            bill_id: value.bill_id,
            mint_node_id: value.mint_node_id,
            mint_request_id: Uuid::from_str(&value.mint_request_id)
                .map_err(|_| Error::EncodingError)?,
            timestamp: value.timestamp,
            status: value.status.into(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MintRequestStatusDb {
    Pending,
    Denied { timestamp: Timestamp },
    Offered,
    Accepted,
    MintingEnabled,
    Rejected { timestamp: Timestamp },
    Cancelled { timestamp: Timestamp },
    Expired { timestamp: Timestamp },
}

impl From<MintRequestStatusDb> for MintRequestStatus {
    fn from(value: MintRequestStatusDb) -> Self {
        match value {
            MintRequestStatusDb::Pending => MintRequestStatus::Pending,
            MintRequestStatusDb::Denied { timestamp } => MintRequestStatus::Denied { timestamp },
            MintRequestStatusDb::Offered => MintRequestStatus::Offered,
            MintRequestStatusDb::Accepted => MintRequestStatus::Accepted,
            MintRequestStatusDb::MintingEnabled => MintRequestStatus::MintingEnabled,
            MintRequestStatusDb::Rejected { timestamp } => {
                MintRequestStatus::Rejected { timestamp }
            }
            MintRequestStatusDb::Cancelled { timestamp } => {
                MintRequestStatus::Cancelled { timestamp }
            }
            MintRequestStatusDb::Expired { timestamp } => MintRequestStatus::Expired { timestamp },
        }
    }
}

impl From<MintRequestStatus> for MintRequestStatusDb {
    fn from(value: MintRequestStatus) -> Self {
        match value {
            MintRequestStatus::Pending => MintRequestStatusDb::Pending,
            MintRequestStatus::Denied { timestamp } => MintRequestStatusDb::Denied { timestamp },
            MintRequestStatus::Offered => MintRequestStatusDb::Offered,
            MintRequestStatus::Accepted => MintRequestStatusDb::Accepted,
            MintRequestStatus::MintingEnabled => MintRequestStatusDb::MintingEnabled,
            MintRequestStatus::Rejected { timestamp } => {
                MintRequestStatusDb::Rejected { timestamp }
            }
            MintRequestStatus::Cancelled { timestamp } => {
                MintRequestStatusDb::Cancelled { timestamp }
            }
            MintRequestStatus::Expired { timestamp } => MintRequestStatusDb::Expired { timestamp },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::get_memory_db,
        tests::tests::{bill_id_test, node_id_test, node_id_test_other, test_ts},
    };
    use uuid::uuid;

    pub fn get_uuid_v4() -> Uuid {
        uuid!("00000000-0000-0000-0000-000000000000")
    }

    async fn get_requests_store() -> SurrealMintStore {
        let mem_db = get_memory_db("test", "requests")
            .await
            .expect("could not create memory db");
        SurrealMintStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    async fn get_offers_store() -> SurrealMintStore {
        let mem_db = get_memory_db("test", "offers")
            .await
            .expect("could not create memory db");
        SurrealMintStore::new(SurrealWrapper {
            db: mem_db,
            files: false,
        })
    }

    #[tokio::test]
    async fn test_exists_for_bill() {
        let store = get_requests_store().await;
        let id = get_uuid_v4();
        assert!(
            !store
                .exists_for_bill(&node_id_test(), &bill_id_test())
                .await
                .unwrap()
        );
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        assert!(
            store
                .exists_for_bill(&node_id_test(), &bill_id_test())
                .await
                .unwrap()
        );
        assert!(
            !store
                .exists_for_bill(&node_id_test_other(), &bill_id_test())
                .await
                .unwrap()
        );
        store
            .add_request(
                &node_id_test_other(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        assert!(
            store
                .exists_for_bill(&node_id_test_other(), &bill_id_test())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_get_requests() {
        let store = get_requests_store().await;
        let reqs = store
            .get_requests(&node_id_test(), &bill_id_test(), &node_id_test_other())
            .await
            .unwrap();
        let id = get_uuid_v4();
        assert_eq!(reqs.len(), 0);
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        let reqs = store
            .get_requests(&node_id_test(), &bill_id_test(), &node_id_test_other())
            .await
            .unwrap();
        assert_eq!(reqs.len(), 1);
    }

    #[tokio::test]
    async fn test_get_requests_for_bill() {
        let store = get_requests_store().await;
        let reqs = store
            .get_requests_for_bill(&node_id_test(), &bill_id_test())
            .await
            .unwrap();
        let id = get_uuid_v4();
        assert_eq!(reqs.len(), 0);
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        let reqs = store
            .get_requests_for_bill(&node_id_test(), &bill_id_test())
            .await
            .unwrap();
        assert_eq!(reqs.len(), 1);
    }

    #[tokio::test]
    async fn test_get_active_requests() {
        let store = get_requests_store().await;
        let id = get_uuid_v4();
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        let reqs = store.get_all_active_requests().await.unwrap();
        assert_eq!(reqs.len(), 1);
    }

    #[tokio::test]
    async fn test_get_request() {
        let store = get_requests_store().await;
        let id = get_uuid_v4();
        let req = store.get_request(&id).await.unwrap();
        assert!(req.is_none());
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        let req = store.get_request(&id).await.unwrap();
        assert!(req.is_some());
    }

    #[tokio::test]
    async fn test_update_request() {
        let store = get_requests_store().await;
        let id = get_uuid_v4();
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        let req = store.get_request(&id).await.unwrap();
        assert!(matches!(
            req.as_ref().unwrap().status,
            MintRequestStatus::Pending
        ));
        store
            .update_request(&id, &MintRequestStatus::Offered)
            .await
            .unwrap();
        let req = store.get_request(&id).await.unwrap();
        assert!(matches!(
            req.as_ref().unwrap().status,
            MintRequestStatus::Offered
        ));
    }

    #[tokio::test]
    async fn test_get_offer() {
        let store = get_requests_store().await;
        let offer_store = get_offers_store().await;
        let id = get_uuid_v4();
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        offer_store
            .add_offer(&id, "keyset_id", test_ts(), Sum::new_sat(1500).unwrap())
            .await
            .unwrap();
        let offer = offer_store.get_offer(&id).await.unwrap();
        assert!(offer.is_some());
    }

    #[tokio::test]
    async fn test_update_offer() {
        let store = get_requests_store().await;
        let offer_store = get_offers_store().await;
        let id = get_uuid_v4();
        store
            .add_request(
                &node_id_test(),
                &bill_id_test(),
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();
        offer_store
            .add_offer(&id, "keyset_id", test_ts(), Sum::new_sat(1500).unwrap())
            .await
            .unwrap();
        let offer = offer_store.get_offer(&id).await.unwrap();
        assert!(offer.is_some());
        assert!(offer.as_ref().unwrap().recovery_data.is_none());

        // recovery data
        offer_store
            .add_recovery_data_to_offer(&id, &["secret".to_owned()], &["r".to_owned()])
            .await
            .unwrap();
        let offer = offer_store.get_offer(&id).await.unwrap();
        assert!(offer.as_ref().unwrap().recovery_data.is_some());
        assert!(offer.as_ref().unwrap().proofs.is_none());

        // proofs
        offer_store
            .add_proofs_to_offer(&id, "proofs")
            .await
            .unwrap();
        let offer = offer_store.get_offer(&id).await.unwrap();
        assert!(offer.as_ref().unwrap().proofs.is_some());
        assert_eq!(offer.as_ref().unwrap().proofs.as_ref().unwrap(), "proofs");
        assert!(!offer.as_ref().unwrap().proofs_spent);

        // proofs spent
        offer_store
            .set_proofs_to_spent_for_offer(&id)
            .await
            .unwrap();
        let offer = offer_store.get_offer(&id).await.unwrap();
        assert!(offer.as_ref().unwrap().proofs_spent);
    }

    #[tokio::test]
    async fn test_dev_mode_reset_for_bill_deletes_requests_and_offers() {
        let store = get_requests_store().await;
        let id = get_uuid_v4();
        let bill_id = bill_id_test();

        store
            .add_request(
                &node_id_test(),
                &bill_id,
                &node_id_test_other(),
                &id,
                test_ts(),
            )
            .await
            .unwrap();

        store
            .add_offer(&id, "keyset_id", test_ts(), Sum::new_sat(1500).unwrap())
            .await
            .unwrap();

        assert!(store.get_request(&id).await.unwrap().is_some());
        assert!(store.get_offer(&id).await.unwrap().is_some());

        store.dev_mode_reset_for_bill(&bill_id).await.unwrap();

        assert!(store.get_request(&id).await.unwrap().is_none());
        assert!(store.get_offer(&id).await.unwrap().is_none());
    }
}
