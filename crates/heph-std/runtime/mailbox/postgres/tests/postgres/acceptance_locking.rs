use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use mailbox_postgres::MailboxPersistenceError;
use serial_test::serial;
use uuid::Uuid;

use super::{
    acceptance_locking_support::{Harness, join},
    support::event,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn duplicate_acceptance_waits_before_child_writes_and_returns_one_receipt() {
    let Some(h) = Harness::connect().await else {
        return;
    };
    let body = b"instance-first-duplicate";
    let mut blocker = h.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id = $1 FOR UPDATE")
        .bind(h.fixture.instance.as_uuid())
        .fetch_one(&mut *blocker)
        .await
        .expect("hold exact instance before concurrent acceptance");
    let first = h.accept(event(h.mailbox, h.fixture.instance, body), body);
    let duplicate = h.accept(event(h.mailbox, h.fixture.instance, body), body);
    h.wait_for_acceptors(2).await;
    assert_eq!(h.counts().await, (0, 0, 0, 0));
    blocker.commit().await.unwrap();
    let first = join(first).await.expect("first serialized acceptance");
    let duplicate = join(duplicate)
        .await
        .expect("duplicate serialized acceptance");
    assert_eq!(first.event_id, duplicate.event_id);
    assert_ne!(first.duplicate, duplicate.duplicate);
    assert_eq!(h.counts().await, (1, 1, 1, 1));
    let mismatch = h
        .store
        .accept(
            Uuid::new_v4(),
            &event(h.mailbox, h.fixture.instance, body),
            body,
            u32::try_from(body.len()).unwrap(),
        )
        .await;
    assert!(matches!(
        mismatch,
        Err(MailboxPersistenceError::Unavailable)
    ));
    assert_eq!(h.counts().await, (1, 1, 1, 1));
    h.settle_outbox(first.event_id.as_uuid()).await;
    h.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn acceptance_waiting_for_permanent_close_is_denied_without_child_writes() {
    let Some(h) = Harness::connect().await else {
        return;
    };
    // The shared fixture grants organization ownership for inspection only.
    // Closure needs the same explicit project manager relationship as the
    // existing actor mailbox allocation test, including UPDATE RLS for locks.
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(h.fixture.project)
        .bind(h.fixture.owner.user_id.as_uuid())
        .execute(&h.pool)
        .await
        .expect("grant actual fixture manager before actor closure");
    let body = b"closure-wins-admission";
    let mut closer = begin_actor_transaction(&h.pool, &h.fixture.owner)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *closer)
        .await
        .unwrap();
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM agent_instances WHERE id = $1 FOR UPDATE")
            .bind(h.fixture.instance.as_uuid())
            .fetch_one(&mut *closer)
            .await
            .unwrap();
    let acceptor = h.accept(event(h.mailbox, h.fixture.instance, body), body);
    h.wait_for_acceptors(1).await;
    assert_eq!(h.counts().await, (0, 0, 0, 0));
    let removal = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_instance_removal_requests
             (id, instance_id, expected_version, input_hash, created_by,
              request_id, authorization_model_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(removal)
    .bind(h.fixture.instance.as_uuid())
    .bind(version)
    .bind([41_u8; 32].as_slice())
    .bind(h.fixture.owner.user_id.as_uuid())
    .bind(h.fixture.owner.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut *closer)
    .await
    .expect("admit exact live manager permanent closure");
    sqlx::query("SELECT * FROM close_instance_launches($1)")
        .bind(removal)
        .fetch_all(&mut *closer)
        .await
        .expect("close mailbox under parent lock");
    closer
        .commit()
        .await
        .expect("commit permanent closure before acceptance resumes");
    assert!(matches!(
        join(acceptor).await,
        Err(MailboxPersistenceError::Unavailable)
    ));
    assert_eq!(h.counts().await, (0, 0, 0, 0));
    assert!(
        h.store
            .accept(
                h.fixture.project,
                &event(h.mailbox, h.fixture.instance, body),
                body,
                u32::try_from(body.len()).unwrap(),
            )
            .await
            .is_err(),
        "closed consumer must remain denied on retry"
    );
    assert_eq!(h.counts().await, (0, 0, 0, 0));
    h.pool.close().await;
}
