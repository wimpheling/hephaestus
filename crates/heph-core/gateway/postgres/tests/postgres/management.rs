//! management scenario.

use super::support::{request, seed_fixture, set_actor_app_role};
use gateway_postgres::{GatewayMailboxPublicationResult, PostgresGatewayMailboxPublisher};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::env;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn gateway_mailbox_management_and_inspection_are_actor_scoped() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway mailbox migrations");

    let fixture = seed_fixture(&pool).await;
    let outsider = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Gateway mailbox outsider')")
        .bind(outsider)
        .execute(&pool)
        .await
        .expect("outsider");

    // First create an accepted publication. It gives the inspection proof a
    // durable, protected row rather than merely checking an empty relation.
    let publisher = PostgresGatewayMailboxPublisher::new(pool.clone());
    assert!(matches!(
        publisher
            .publish(request(&fixture, "deliver", "actor-scoped-inspection"))
            .await
            .expect("accept publication for inspection"),
        GatewayMailboxPublicationResult::Accepted { .. }
    ));

    let managed_binding = Uuid::new_v4();
    let managed_grant = Uuid::new_v4();
    let mut owner_tx = pool.begin().await.expect("owner transaction");
    set_actor_app_role(&mut owner_tx, fixture.owner)
        .await
        .expect("owner application role");
    sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'gateway-managed-proof', $6)",
    )
    .bind(managed_binding)
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&mut *owner_tx)
    .await
    .expect("authorized actor creates exact binding");
    sqlx::query(
        "INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
         VALUES ($1, $2, 'active', $3)",
    )
    .bind(managed_grant)
    .bind(managed_binding)
    .bind(fixture.owner)
    .execute(&mut *owner_tx)
    .await
    .expect("authorized actor creates exact grant");
    let visible_to_owner: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_mailbox_bindings WHERE id = $1")
            .bind(managed_binding)
            .fetch_one(&mut *owner_tx)
            .await
            .expect("owner can inspect binding");
    let publication_visible_to_owner: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_mailbox_publications WHERE invocation_id = $1",
    )
    .bind(fixture.invocation)
    .fetch_one(&mut *owner_tx)
    .await
    .expect("owner can inspect publication provenance");
    assert_eq!(visible_to_owner, 1);
    assert_eq!(publication_visible_to_owner, 1);
    owner_tx.commit().await.expect("commit authorized binding");

    let mut outsider_tx = pool.begin().await.expect("outsider transaction");
    set_actor_app_role(&mut outsider_tx, outsider)
        .await
        .expect("outsider application role");
    let bindings_visible_to_outsider: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_mailbox_bindings WHERE id = $1")
            .bind(managed_binding)
            .fetch_one(&mut *outsider_tx)
            .await
            .expect("outsider binding inspection is filtered");
    let publications_visible_to_outsider: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM gateway_mailbox_publications WHERE invocation_id = $1",
    )
    .bind(fixture.invocation)
    .fetch_one(&mut *outsider_tx)
    .await
    .expect("outsider publication inspection is filtered");
    assert_eq!(bindings_visible_to_outsider, 0);
    assert_eq!(publications_visible_to_outsider, 0);
    let forbidden_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1",
    )
    .bind(managed_grant)
    .bind(outsider)
    .execute(&mut *outsider_tx)
    .await
    .expect("outsider revoke is RLS-filtered");
    assert_eq!(forbidden_revoke.rows_affected(), 0);
    outsider_tx
        .rollback()
        .await
        .expect("rollback outsider checks");

    // A failed insert aborts its PostgreSQL transaction, so keep it separate
    // from the RLS-filtered revocation assertion above.
    let mut outsider_create_tx = pool.begin().await.expect("outsider create transaction");
    set_actor_app_role(&mut outsider_create_tx, outsider)
        .await
        .expect("outsider application role");
    let forbidden_create = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'outsider-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(outsider)
    .execute(&mut *outsider_create_tx)
    .await;
    assert!(
        forbidden_create.is_err(),
        "outsider cannot create a binding"
    );
    outsider_create_tx
        .rollback()
        .await
        .expect("rollback rejected outsider create");

    let mut worker_mint_tx = pool.begin().await.expect("worker mint transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *worker_mint_tx)
        .await
        .expect("worker role");
    let worker_mint = sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
             (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
         VALUES ($1, $2, $3, $4, 'other', $5, 'worker-proof', $6)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.revision)
    .bind(fixture.gateway)
    .bind(fixture.project)
    .bind(fixture.mailbox)
    .bind(fixture.owner)
    .execute(&mut *worker_mint_tx)
    .await;
    assert!(worker_mint.is_err(), "worker cannot mint a binding");
    worker_mint_tx
        .rollback()
        .await
        .expect("rollback rejected worker mint");

    let mut worker_revoke_tx = pool.begin().await.expect("worker revoke transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *worker_revoke_tx)
        .await
        .expect("worker role");
    let worker_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1",
    )
    .bind(managed_grant)
    .bind(fixture.owner)
    .execute(&mut *worker_revoke_tx)
    .await;
    assert!(worker_revoke.is_err(), "worker cannot revoke a grant");
    worker_revoke_tx
        .rollback()
        .await
        .expect("rollback rejected worker revoke");

    let mut owner_revoke_tx = pool.begin().await.expect("owner revoke transaction");
    set_actor_app_role(&mut owner_revoke_tx, fixture.owner)
        .await
        .expect("owner application role");
    let authorized_revoke = sqlx::query(
        "UPDATE gateway_mailbox_binding_grants
             SET status = 'revoked', revoked_at = now(), revoked_by = $2
           WHERE id = $1 AND status = 'active'",
    )
    .bind(managed_grant)
    .bind(fixture.owner)
    .execute(&mut *owner_revoke_tx)
    .await
    .expect("authorized actor revokes grant");
    assert_eq!(authorized_revoke.rows_affected(), 1);
    owner_revoke_tx
        .commit()
        .await
        .expect("commit authorized revocation");
}
