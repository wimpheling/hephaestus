//! Actual current-manager allocation/control and trusted external worker ingress.

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use mailbox_dispatch::{
    MAILBOX_CANCEL_SUBJECT, MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand,
    MailboxDispatchStore,
};
use mailbox_domain::{MailboxEventId, MailboxId};
use mailbox_postgres::{MailboxOperatorAction, PostgresMailboxRepository};
use run_domain::{FreshRunExecutionProfile, StartRun};
use sqlx::PgPool;
use uuid::Uuid;

#[path = "event.rs"]
mod event;

pub(super) struct Fixture {
    pub(super) original: crate::fixture::Fixture,
    store: PostgresMailboxRepository,
    pub(super) start: StartRun,
    pub(super) event: MailboxEventId,
    mailbox: MailboxId,
    actor: AuthenticatedIdentity,
    control: PostgresMailboxRepository,
}

impl Fixture {
    pub(super) async fn new(pool: &PgPool) -> Self {
        let original = crate::fixture::Fixture::new(pool).await;
        let (actor, project) = manager(pool, &original).await;
        let control = PostgresMailboxRepository::new(original.application.clone());
        let mailbox = control
            .allocate(&actor, original.start.instance_id)
            .await
            .unwrap();
        assert_eq!(
            control
                .allocate(&actor, original.start.instance_id)
                .await
                .unwrap(),
            mailbox
        );
        let store = PostgresMailboxRepository::new(original.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
            .with_legacy_vm_placement(original.scope.clone());
        let incoming = event::external(mailbox, original.start.instance_id);
        let accepted = store
            .accept(
                project,
                &incoming,
                event::BODY,
                u32::try_from(event::BODY.len()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.event_id, incoming.id);
        assert!(!accepted.duplicate);
        let wake = committed(pool, accepted.event_id, MAILBOX_WAKE_SUBJECT).await;
        store
            .apply_command(MAILBOX_WAKE_SUBJECT, &wake)
            .await
            .unwrap();
        let dispatch = committed(pool, accepted.event_id, MAILBOX_DISPATCH_SUBJECT).await;
        store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch)
            .await
            .unwrap();
        let start = store.claim_dispatch(&dispatch).await.unwrap().unwrap();
        assert!(store.claim_dispatch(&dispatch).await.unwrap().is_none());
        assert_eq!(start.instance_id, original.start.instance_id);
        assert_eq!(
            start.instance_revision_id,
            original.start.instance_revision_id
        );
        assert_eq!(start.attachment_id, original.start.attachment_id);
        assert!(!start.requires_state);
        Self {
            original,
            store,
            start,
            event: accepted.event_id,
            mailbox,
            actor,
            control,
        }
    }

    pub(super) async fn cancel_delivery(&self, pool: &PgPool) {
        assert!(
            self.control
                .operate(
                    &self.actor,
                    MailboxOperatorAction::Cancel,
                    self.mailbox,
                    Some(self.event)
                )
                .await
                .unwrap()
                .changed
        );
        let cancel = committed(pool, self.event, MAILBOX_CANCEL_SUBJECT).await;
        self.store
            .apply_command(MAILBOX_CANCEL_SUBJECT, &cancel)
            .await
            .unwrap();
        // Cancellation is terminal eligibility, not proof that its Run has stopped.
        let dispatch = committed(pool, self.event, MAILBOX_DISPATCH_SUBJECT).await;
        assert!(
            self.store
                .claim_dispatch(&dispatch)
                .await
                .unwrap()
                .is_none()
        );
        let actual: (String, bool, i64) = sqlx::query_as("SELECT disposition,terminal_at IS NOT NULL,(SELECT count(*) FROM mailbox_operator_audit WHERE mailbox_id=$2 AND event_id=$1 AND actor_id=$3 AND action='cancel' AND changed) FROM mailbox_deliveries WHERE event_id=$1")
            .bind(self.event.as_uuid()).bind(self.mailbox.as_uuid()).bind(self.actor.user_id.as_uuid())
            .fetch_one(pool).await.unwrap();
        assert_eq!(actual, ("cancelled".into(), true, 1));
    }
}

async fn committed(pool: &PgPool, event: MailboxEventId, subject: &str) -> MailboxDispatchCommand {
    let mut values: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM outbox WHERE subject=$1 AND payload->>'mailbox_event_id'=$2",
    )
    .bind(subject)
    .bind(event.to_string())
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(values.len(), 1, "one actual committed {subject} operation");
    serde_json::from_value(values.pop().unwrap()).unwrap()
}

async fn manager(
    pool: &PgPool,
    original: &crate::fixture::Fixture,
) -> (AuthenticatedIdentity, Uuid) {
    let project: Uuid = sqlx::query_scalar("SELECT project_id FROM repositories WHERE id=$1")
        .bind(original.repository)
        .fetch_one(pool)
        .await
        .unwrap();
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Mailbox history manager')")
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    (
        AuthenticatedIdentity::new(
            UserId::from_uuid(user),
            "ordinary-mailbox-history",
            user.to_string(),
            serde_json::json!({}),
            RequestId::new(),
        ),
        project,
    )
}
