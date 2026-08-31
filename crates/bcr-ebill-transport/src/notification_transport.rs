use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_common::wire::quotes::ApplicantActionProjection;

use bcr_ebill_api::external::email::EmailClientApi;
use bcr_ebill_api::service::transport_service::NotificationTransportServiceApi;
use bcr_ebill_api::service::transport_service::{Error, Result};
use bcr_ebill_api::util::{validate_bill_id_network, validate_node_id_network};
use bcr_ebill_core::application::ServiceTraitBounds;
use bcr_ebill_core::application::notification::{
    Notification, NotificationLevel, NotificationType, QuoteApplicantActionNotificationPayload,
};
use bcr_ebill_core::{
    protocol::Sum,
    protocol::blockchain::bill::participant::BillParticipant,
    protocol::event::{ActionType, BillChainEventPayload, BillEventType, Event},
};
use bcr_ebill_persistence::NotificationStoreApi;
use bcr_ebill_persistence::notification::{EmailNotificationStoreApi, NotificationFilter};
use log::{debug, error};

use crate::PushApi;

const QUOTE_APPLICANT_ACTION_NOTIFICATION_DESCRIPTION: &str = "quote_applicant_action";

pub struct NotificationTransportService {
    notification_store: Arc<dyn NotificationStoreApi>,
    email_notification_store: Arc<dyn EmailNotificationStoreApi>,
    #[allow(unused)]
    email_client: Arc<dyn EmailClientApi>,
    push_service: Arc<dyn PushApi>,
}

impl NotificationTransportService {
    pub fn new(
        notification_store: Arc<dyn NotificationStoreApi>,
        email_notification_store: Arc<dyn EmailNotificationStoreApi>,
        email_client: Arc<dyn EmailClientApi>,
        push_service: Arc<dyn PushApi>,
    ) -> Self {
        Self {
            notification_store,
            email_notification_store,
            email_client,
            push_service,
        }
    }

    async fn create_bill_notification(
        &self,
        node_id: &NodeId,
        bill_id: &BillId,
        event_type: BillEventType,
        action_type: Option<ActionType>,
        sum: Option<Sum>,
    ) -> Result<()> {
        debug!(
            "Local bill notification: type={:?} bill_id={} recipient={} action={:?}",
            event_type, bill_id, node_id, action_type
        );
        let payload = BillChainEventPayload {
            event_type: event_type.clone(),
            bill_id: bill_id.to_owned(),
            action_type,
            sum,
            sender_node_id: None,
            sender_name: None,
        };

        let notification = Notification::new_bill_notification(
            bill_id,
            node_id,
            &event_type.description(),
            Some(
                serde_json::to_value(&payload)
                    .map_err(|e| Error::Message(format!("Failed to serialize payload: {e}")))?,
            ),
            NotificationLevel::Informational,
        );

        if let Ok(Some(currently_active)) = self
            .notification_store
            .get_latest_by_reference_and_node_id(
                &bill_id.to_string(),
                NotificationType::Bill,
                node_id,
            )
            .await
            && quote_applicant_action_payload(&currently_active).is_none()
        {
            let _ = self
                .notification_store
                .mark_as_done(&currently_active.id)
                .await;
        }

        match self.notification_store.add(notification.clone()).await {
            Ok(_) => {
                if let Ok(notification_value) = serde_json::to_value(notification) {
                    self.push_service.send(notification_value).await;
                }
            }
            Err(e) => {
                error!("Failed to save bill notification: {e}");
                return Err(Error::Persistence(
                    "Failed to save bill notification".to_string(),
                ));
            }
        }

        Ok(())
    }

    async fn mark_done_and_push(&self, mut notification: Notification) -> Result<()> {
        self.notification_store
            .mark_as_done(&notification.id)
            .await
            .map_err(|e| {
                error!("Failed to mark quote applicant-action notification as done: {e}");
                Error::Persistence(
                    "Failed to mark quote applicant-action notification as done".to_string(),
                )
            })?;
        notification.active = false;
        if let Ok(notification_value) = serde_json::to_value(notification) {
            self.push_service.send(notification_value).await;
        }
        Ok(())
    }
}

fn quote_applicant_action_payload(
    notification: &Notification,
) -> Option<QuoteApplicantActionNotificationPayload> {
    if notification.notification_type != NotificationType::Bill {
        return None;
    }
    let payload: QuoteApplicantActionNotificationPayload =
        serde_json::from_value(notification.payload.clone()?).ok()?;
    payload.is_current_schema().then_some(payload)
}

impl ServiceTraitBounds for NotificationTransportService {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl NotificationTransportServiceApi for NotificationTransportService {
    async fn get_client_notifications(
        &self,
        filter: NotificationFilter,
    ) -> Result<Vec<Notification>> {
        for node_id in filter.node_ids.iter() {
            validate_node_id_network(node_id)?;
        }
        let result = self.notification_store.list(filter).await.map_err(|e| {
            error!("Failed to get client notifications: {e}");
            Error::Persistence("Failed to get client notifications".to_string())
        })?;
        Ok(result)
    }

    async fn mark_notification_as_done(&self, notification_id: &str) -> Result<()> {
        let _ = self
            .notification_store
            .mark_as_done(notification_id)
            .await
            .map_err(|e| {
                error!("Failed to mark notification as done: {e}");
                Error::Persistence("Failed to mark notification as done".to_string())
            })?;
        Ok(())
    }

    async fn get_active_bill_notification(&self, bill_id: &BillId) -> Option<Notification> {
        validate_bill_id_network(bill_id).ok()?;
        self.notification_store
            .get_latest_by_reference(&bill_id.to_string(), NotificationType::Bill)
            .await
            .unwrap_or_default()
    }

    async fn get_active_bill_notifications(
        &self,
        bill_ids: &[BillId],
    ) -> HashMap<BillId, Notification> {
        let ids: Vec<String> = bill_ids.iter().map(|bill_id| bill_id.to_string()).collect();
        let refs = self
            .notification_store
            .get_latest_by_references(&ids, NotificationType::Bill)
            .await
            .unwrap_or_default();
        refs.into_iter()
            .filter_map(|(key, value)| match BillId::from_str(&key) {
                Ok(bill_id) => Some((bill_id, value)),
                Err(_) => None,
            })
            .collect()
    }

    async fn get_active_notification_status_for_node_ids(
        &self,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, bool>> {
        Ok(self
            .notification_store
            .get_active_status_for_node_ids(node_ids)
            .await
            .unwrap_or_default())
    }

    async fn create_local_bill_notification(
        &self,
        node_id: &NodeId,
        bill_id: &BillId,
        event_type: BillEventType,
        action_type: Option<ActionType>,
        sum: Option<Sum>,
    ) -> Result<()> {
        self.create_bill_notification(node_id, bill_id, event_type, action_type, sum)
            .await
    }

    async fn reconcile_quote_applicant_action_notification(
        &self,
        node_id: &NodeId,
        bill_id: &BillId,
        mint_request_id: uuid::Uuid,
        applicant_action: Option<ApplicantActionProjection>,
    ) -> Result<()> {
        validate_node_id_network(node_id)?;
        validate_bill_id_network(bill_id)?;
        let notifications = self
            .notification_store
            .list(NotificationFilter {
                reference_id: Some(bill_id.to_string()),
                notification_type: Some(NotificationType::Bill.to_string()),
                node_ids: vec![node_id.to_owned()],
                ..Default::default()
            })
            .await
            .map_err(|e| {
                error!("Failed to load quote applicant-action notifications: {e}");
                Error::Persistence(
                    "Failed to load quote applicant-action notifications".to_string(),
                )
            })?;
        let action_notifications = notifications
            .into_iter()
            .filter_map(|notification| {
                quote_applicant_action_payload(&notification).map(|payload| (notification, payload))
            })
            .collect::<Vec<_>>();

        if let Some(action) = applicant_action {
            let revision_already_seen = action_notifications.iter().any(|(_, payload)| {
                payload.mint_request_id == mint_request_id && payload.applicant_action == action
            });

            for (notification, payload) in action_notifications {
                if notification.active
                    && payload.mint_request_id == mint_request_id
                    && payload.applicant_action != action
                {
                    self.mark_done_and_push(notification).await?;
                }
            }

            // A dismissed revision stays dismissed, but it must not leave an older revision active.
            if revision_already_seen {
                return Ok(());
            }

            let payload =
                QuoteApplicantActionNotificationPayload::new(bill_id, mint_request_id, action);
            let notification = Notification::new_bill_notification(
                bill_id,
                node_id,
                QUOTE_APPLICANT_ACTION_NOTIFICATION_DESCRIPTION,
                Some(serde_json::to_value(payload).map_err(|e| {
                    Error::Message(format!(
                        "Failed to serialize quote applicant-action notification: {e}"
                    ))
                })?),
                NotificationLevel::ActionRequired,
            );
            self.notification_store
                .add(notification.clone())
                .await
                .map_err(|e| {
                    error!("Failed to save quote applicant-action notification: {e}");
                    Error::Persistence(
                        "Failed to save quote applicant-action notification".to_string(),
                    )
                })?;
            if let Ok(notification_value) = serde_json::to_value(notification) {
                self.push_service.send(notification_value).await;
            }
        } else {
            for (notification, payload) in action_notifications {
                if notification.active && payload.mint_request_id == mint_request_id {
                    self.mark_done_and_push(notification).await?;
                }
            }
        }
        Ok(())
    }
    async fn create_general_notification(
        &self,
        node_id: &NodeId,
        description: &str,
        reference_id: Option<String>,
        level: NotificationLevel,
    ) -> Result<()> {
        let notification =
            Notification::new_general_notification(node_id, description, reference_id, level);

        self.notification_store
            .add(notification.clone())
            .await
            .map_err(|e| {
                error!("Failed to save general notification: {e}");
                Error::Persistence("Failed to save general notification".to_string())
            })?;

        match serde_json::to_value(notification) {
            Ok(notification_value) => {
                self.push_service.send(notification_value).await;
            }
            Err(e) => {
                error!("Failed to serialize general notification for push: {e}");
            }
        }
        Ok(())
    }

    async fn send_request_to_action_timed_out_event(
        &self,
        sender_node_id: &NodeId,
        bill_id: &BillId,
        sum: Option<Sum>,
        timed_out_action: ActionType,
        recipients: Vec<BillParticipant>,
        holder: &NodeId,
        drawee: &NodeId,
        recoursee: &Option<NodeId>,
    ) -> Result<()> {
        if let Some(event_type) = timed_out_action.get_timeout_event_type() {
            let recipient_ids: Vec<NodeId> =
                recipients.iter().map(|r| r.node_id().clone()).collect();

            if !recipient_ids.contains(sender_node_id) {
                return Ok(());
            }

            self.create_bill_notification(
                sender_node_id,
                bill_id,
                event_type.clone(),
                Some(ActionType::CheckBill),
                sum.clone(),
            )
            .await?;

            let payload = BillChainEventPayload {
                event_type,
                bill_id: bill_id.to_owned(),
                action_type: Some(ActionType::CheckBill),
                sum,
                sender_node_id: Some(sender_node_id.clone()),
                sender_name: None,
            };
            let event = Event::new_bill(payload);
            if let Some(r) = recoursee {
                if sender_node_id == r {
                    self.send_email_notification(sender_node_id, holder, &event)
                        .await;
                }
            } else if sender_node_id == drawee {
                self.send_email_notification(sender_node_id, holder, &event)
                    .await;
            }
        }
        Ok(())
    }

    async fn check_bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action: ActionType,
    ) -> Result<bool> {
        validate_bill_id_network(bill_id)?;
        Ok(self
            .notification_store
            .bill_notification_sent(bill_id, block_height, action)
            .await
            .map_err(|e| {
                error!("Failed to check if bill notification was already sent: {e}");
                Error::Persistence(
                    "Failed to check if bill notification was already sent".to_string(),
                )
            })?)
    }

    /// Stores that a notification was sent for the given bill id and action
    async fn mark_bill_notification_sent(
        &self,
        bill_id: &BillId,
        block_height: i32,
        action: ActionType,
    ) -> Result<()> {
        validate_bill_id_network(bill_id)?;
        self.notification_store
            .set_bill_notification_sent(bill_id, block_height, action)
            .await
            .map_err(|e| {
                error!("Failed to mark bill notification as sent: {e}");
                Error::Persistence("Failed to mark bill notification as sent".to_string())
            })?;
        Ok(())
    }

    /// Fetch email notifications preferences link for the currently selected identity
    async fn get_email_notifications_preferences_link(&self, node_id: &NodeId) -> Result<url::Url> {
        match self
            .email_notification_store
            .get_email_preferences_link_for_node_id(node_id)
            .await
        {
            Ok(Some(link)) => Ok(link),
            Ok(None) => Err(Error::NotFound),
            Err(e) => Err(Error::Persistence(e.to_string())),
        }
    }

    /// Attempts to send an email notification for an event to the receiver
    /// if the receiver does not have email notifications enabled, the relay
    /// ignores the request and returns a quick 200 OK.
    async fn send_email_notification(
        &self,
        _sender: &NodeId,
        _receiver: &NodeId,
        _event: &Event<BillChainEventPayload>,
    ) {
        // TODO: use email client and receiver's mint_url to send
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bcr_common::core::{BillId, NodeId};
    use bcr_common::wire::quotes::{ApplicantActionKind, ApplicantActionProjection};
    use bcr_ebill_api::service::transport_service::NotificationTransportServiceApi;
    use bcr_ebill_core::{
        application::notification::{
            Notification, NotificationLevel, QuoteApplicantActionNotificationPayload,
        },
        protocol::Email,
        protocol::Sum,
        protocol::blockchain::bill::participant::BillParticipant,
        protocol::crypto::BcrKeys,
        protocol::event::ActionType,
    };
    use bcr_ebill_persistence::notification::NotificationFilter;
    use mockall::predicate::{eq, function};
    use uuid::Uuid;

    use super::{QUOTE_APPLICANT_ACTION_NOTIFICATION_DESCRIPTION, quote_applicant_action_payload};

    use crate::{
        notification_transport::NotificationTransportService,
        push_notification::MockPushApi,
        test_utils::{
            MockEmailClient, MockEmailNotificationStore, MockNotificationStore, bill_id_test,
            get_identity_public_data, init_test_cfg, node_id_test, node_id_test_other,
            node_id_test_other2,
        },
    };

    use bcr_ebill_api::service::transport_service::Error;

    #[tokio::test]
    async fn test_send_request_to_action_timed_out_event() {
        init_test_cfg();
        let recipients = vec![
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test(),
                &Email::new("part1@example.com").unwrap(),
                vec![],
            )),
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test_other(),
                &Email::new("part2@example.com").unwrap(),
                vec![],
            )),
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test_other2(),
                &Email::new("part3@example.com").unwrap(),
                vec![],
            )),
        ];

        let service = expect_service(|mock_store, _, email_client, mock_push| {
            mock_store.expect_add().returning(Ok).times(2);
            mock_store
                .expect_get_latest_by_reference_and_node_id()
                .returning(|_, _, _| Ok(None))
                .times(2);

            mock_push.expect_send().returning(|_| ()).times(2);

            email_client
                .expect_send_bill_notification()
                .returning(|_, _, _, _, _, _, _| Ok(()));
        });

        service
            .send_request_to_action_timed_out_event(
                &node_id_test(),
                &bill_id_test(),
                Some(Sum::new_sat(100).expect("sat works")),
                ActionType::PayBill,
                recipients.clone(),
                &node_id_test(),
                &node_id_test(),
                &None,
            )
            .await
            .expect("failed to send event");

        service
            .send_request_to_action_timed_out_event(
                &node_id_test(),
                &bill_id_test(),
                Some(Sum::new_sat(100).expect("sat works")),
                ActionType::AcceptBill,
                recipients.clone(),
                &node_id_test(),
                &node_id_test(),
                &None,
            )
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn test_send_request_to_action_timed_out_does_not_send_non_timeout_action() {
        init_test_cfg();
        let recipients = vec![
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test(),
                &Email::new("part1@example.com").unwrap(),
                vec![],
            )),
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test_other(),
                &Email::new("part2@example.com").unwrap(),
                vec![],
            )),
            BillParticipant::Ident(get_identity_public_data(
                &node_id_test_other2(),
                &Email::new("part3@example.com").unwrap(),
                vec![],
            )),
        ];

        let service = expect_service(|_, _, _, _| {});

        service
            .send_request_to_action_timed_out_event(
                &node_id_test(),
                &bill_id_test(),
                Some(Sum::new_sat(100).expect("sat works")),
                ActionType::CheckBill,
                recipients.clone(),
                &node_id_test(),
                &node_id_test(),
                &None,
            )
            .await
            .expect("failed to send event");
    }

    #[tokio::test]
    async fn get_client_notifications() {
        init_test_cfg();
        let result = Notification::new_bill_notification(
            &bill_id_test(),
            &node_id_test(),
            "desc",
            None,
            NotificationLevel::Informational,
        );
        let filter = NotificationFilter {
            active: Some(true),
            ..Default::default()
        };

        let service = expect_service(|mock_store, _, _, _| {
            let returning = result.clone();
            mock_store
                .expect_list()
                .with(eq(filter.clone()))
                .returning(move |_| Ok(vec![returning.clone()]));
        });

        let res = service
            .get_client_notifications(filter)
            .await
            .expect("could not get notifications");
        assert!(!res.is_empty());
        assert_eq!(res[0].id, result.id);
    }

    #[tokio::test]
    async fn wrong_network_failures() {
        init_test_cfg();
        let mainnet_node_id = NodeId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        let mainnet_bill_id = BillId::new(BcrKeys::new().pub_key(), bitcoin::Network::Bitcoin);
        let filter = NotificationFilter {
            node_ids: vec![mainnet_node_id.clone()],
            ..Default::default()
        };

        let service = expect_service(|_, _, _, _| {});

        assert!(service.get_client_notifications(filter).await.is_err());
        assert!(
            service
                .check_bill_notification_sent(&mainnet_bill_id, 0, ActionType::CheckBill)
                .await
                .is_err()
        );
        assert!(
            service
                .mark_bill_notification_sent(&mainnet_bill_id, 0, ActionType::CheckBill)
                .await
                .is_err()
        );
        assert!(
            service
                .get_active_bill_notification(&mainnet_bill_id)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn get_mark_notification_done() {
        init_test_cfg();

        let service = expect_service(|mock_store, _, _, _| {
            mock_store
                .expect_mark_as_done()
                .with(eq("notification_id"))
                .returning(|_| Ok(()));
        });

        service
            .mark_notification_as_done("notification_id")
            .await
            .expect("could not mark notification as done");
    }

    fn clarification_action(digest_byte: char) -> ApplicantActionProjection {
        ApplicantActionProjection {
            kind: ApplicantActionKind::Clarification,
            revision_digest: format!("sha256:{}", digest_byte.to_string().repeat(64)),
        }
    }

    fn quote_action_notification(
        mint_request_id: Uuid,
        action: ApplicantActionProjection,
    ) -> Notification {
        Notification::new_bill_notification(
            &bill_id_test(),
            &node_id_test(),
            QUOTE_APPLICANT_ACTION_NOTIFICATION_DESCRIPTION,
            Some(
                serde_json::to_value(QuoteApplicantActionNotificationPayload::new(
                    &bill_id_test(),
                    mint_request_id,
                    action,
                ))
                .unwrap(),
            ),
            NotificationLevel::ActionRequired,
        )
    }

    #[tokio::test]
    async fn dismissed_quote_applicant_action_is_not_recreated_for_same_revision() {
        init_test_cfg();
        let quote_id = Uuid::new_v4();
        let action = clarification_action('a');
        let mut current = quote_action_notification(quote_id, action.clone());
        current.active = false;
        let service = expect_service(|store, _, _, _| {
            let dismissed = current.clone();
            store
                .expect_list()
                .return_once(move |_| Ok(vec![dismissed]));
        });

        service
            .reconcile_quote_applicant_action_notification(
                &node_id_test(),
                &bill_id_test(),
                quote_id,
                Some(action),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dismissed_applicant_action_current_revision_clears_active_stale_revision() {
        init_test_cfg();
        let quote_id = Uuid::new_v4();
        let stale = quote_action_notification(quote_id, clarification_action('a'));
        let stale_id = stale.id.clone();
        let current_action = clarification_action('b');
        let mut dismissed_current = quote_action_notification(quote_id, current_action.clone());
        dismissed_current.active = false;
        let service = expect_service(|store, _, _, push| {
            let stale = stale.clone();
            let dismissed_current = dismissed_current.clone();
            store
                .expect_list()
                .return_once(move |_| Ok(vec![stale, dismissed_current]));
            store
                .expect_mark_as_done()
                .with(eq(stale_id.clone()))
                .returning(|_| Ok(()));
            push.expect_send().once().returning(|_| ());
        });

        service
            .reconcile_quote_applicant_action_notification(
                &node_id_test(),
                &bill_id_test(),
                quote_id,
                Some(current_action),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn quote_applicant_action_supersedes_revision_and_clears_when_absent() {
        init_test_cfg();
        let quote_id = Uuid::new_v4();
        let previous = quote_action_notification(quote_id, clarification_action('a'));
        let previous_id = previous.id.clone();
        let unrelated = quote_action_notification(Uuid::new_v4(), clarification_action('c'));
        let next_action = clarification_action('b');
        let expected_next_action = next_action.clone();
        let service = expect_service(|store, _, _, push| {
            let previous = previous.clone();
            let unrelated = unrelated.clone();
            store
                .expect_list()
                .return_once(move |_| Ok(vec![previous, unrelated]));
            store
                .expect_mark_as_done()
                .with(eq(previous_id.clone()))
                .returning(|_| Ok(()));
            let expected_next_action = expected_next_action.clone();
            store
                .expect_add()
                .with(function(move |notification: &Notification| {
                    quote_applicant_action_payload(notification).is_some_and(|payload| {
                        payload.mint_request_id == quote_id
                            && payload.applicant_action == expected_next_action
                    })
                }))
                .returning(Ok);
            push.expect_send().times(2).returning(|_| ());
        });

        service
            .reconcile_quote_applicant_action_notification(
                &node_id_test(),
                &bill_id_test(),
                quote_id,
                Some(next_action),
            )
            .await
            .unwrap();

        let current = quote_action_notification(quote_id, clarification_action('b'));
        let current_id = current.id.clone();
        let service = expect_service(|store, _, _, push| {
            let current = current.clone();
            store.expect_list().return_once(move |_| Ok(vec![current]));
            store
                .expect_mark_as_done()
                .with(eq(current_id.clone()))
                .returning(|_| Ok(()));
            push.expect_send().once().returning(|_| ());
        });
        service
            .reconcile_quote_applicant_action_notification(
                &node_id_test(),
                &bill_id_test(),
                quote_id,
                None,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_get_email_notifications_preferences_link() {
        init_test_cfg();

        let service = expect_service(|_, email_store, _, _| {
            email_store
                .expect_get_email_preferences_link_for_node_id()
                .returning(|_| Ok(Some(url::Url::parse("http://bit.cr/").unwrap())))
                .times(1);
        });

        let result = service
            .get_email_notifications_preferences_link(&node_id_test())
            .await;
        assert!(result.is_ok());
        assert_eq!(
            result.as_ref().unwrap(),
            &url::Url::parse("http://bit.cr/").unwrap()
        );
    }

    #[tokio::test]
    async fn test_get_email_notifications_preferences_link_no_entry() {
        init_test_cfg();
        let service = expect_service(|_, email_store, _, _| {
            email_store
                .expect_get_email_preferences_link_for_node_id()
                .returning(|_| Ok(None))
                .times(1);
        });
        let result = service
            .get_email_notifications_preferences_link(&node_id_test())
            .await;
        assert!(result.is_err());
        assert!(matches!(result, Err(Error::NotFound)));
    }

    fn get_mocks() -> (
        MockNotificationStore,
        MockEmailNotificationStore,
        MockEmailClient,
        MockPushApi,
    ) {
        (
            MockNotificationStore::new(),
            MockEmailNotificationStore::new(),
            MockEmailClient::new(),
            MockPushApi::new(),
        )
    }
    fn get_transport(
        notification_store: MockNotificationStore,
        email_notification_store: MockEmailNotificationStore,
        email_client: MockEmailClient,
        push_service: MockPushApi,
    ) -> NotificationTransportService {
        NotificationTransportService::new(
            Arc::new(notification_store),
            Arc::new(email_notification_store),
            Arc::new(email_client),
            Arc::new(push_service),
        )
    }

    fn expect_service(
        expect: impl Fn(
            &mut MockNotificationStore,
            &mut MockEmailNotificationStore,
            &mut MockEmailClient,
            &mut MockPushApi,
        ),
    ) -> NotificationTransportService {
        let (
            mut notification_store,
            mut email_notification_store,
            mut email_client,
            mut push_service,
        ) = get_mocks();
        expect(
            &mut notification_store,
            &mut email_notification_store,
            &mut email_client,
            &mut push_service,
        );
        get_transport(
            notification_store,
            email_notification_store,
            email_client,
            push_service,
        )
    }
}
