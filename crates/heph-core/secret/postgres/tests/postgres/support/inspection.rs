use super::Fixture;
use super::identity;
use runtime_types::RunId;
use secret_domain::SecretVersionId;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn assert_https_inspection(
    pool: &PgPool,
    fixture: &Fixture,
    run_id: RunId,
    version: SecretVersionId,
) {
    let mut ids = [Uuid::new_v4(), Uuid::new_v4()];
    ids.sort_unstable();
    for id in ids {
        sqlx::query(
            "INSERT INTO brokered_secret_audit_events
             (id, lease_snapshot_id, rule_id, runtime_session_id, run_id,
              request_id, event_kind, decision, occurred_at)
             SELECT $1, id, rule_id, runtime_session_id, run_id,
                    $1, 'authorization_decision', 'allow', now()
             FROM brokered_secret_lease_snapshots WHERE run_id = $2",
        )
        .bind(id)
        .bind(run_id.as_uuid())
        .execute(pool)
        .await
        .expect("seed audit evidence");
    }
    for (user, visible) in [
        (fixture.owner, true),
        (fixture.ordinary_member, false),
        (fixture.other_owner, false),
    ] {
        let mut tx = authz_postgres::begin_actor_transaction(pool, &identity(user))
            .await
            .expect("actor transaction");
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *tx)
            .await
            .expect("exercise application privileges");
        let first: Vec<(Uuid, Uuid)> =
            sqlx::query_as("SELECT id, secret_version_id FROM inspect_run_https_uses($1, NULL, 1)")
                .bind(run_id.as_uuid())
                .fetch_all(&mut *tx)
                .await
                .expect("inspect first page");
        if visible {
            assert_eq!(first, vec![(ids[0], version.as_uuid())]);
            let second: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM inspect_run_https_uses($1, $2, 1)")
                    .bind(run_id.as_uuid())
                    .bind(ids[0])
                    .fetch_all(&mut *tx)
                    .await
                    .expect("inspect next page");
            assert_eq!(second, vec![ids[1]]);
        } else {
            assert!(
                first.is_empty(),
                "run access alone must not reveal secret metadata"
            );
        }
        tx.rollback().await.expect("close inspection");
    }
}
