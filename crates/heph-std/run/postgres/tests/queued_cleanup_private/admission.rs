//! Raw owning boundaries reject wrong roles/isolation before any lock.
use super::{
    observations::{code, rows},
    queued_fixture::Queued,
};
use sqlx::{PgPool, Postgres, Transaction};

pub fn original(h: &Queued) -> serde_json::Value {
    let p = h.request.original();
    serde_json::json!({"drain_attempt":p.drain_attempt,"drain_command":p.drain_command,
        "removal_id":p.removal_id,"command_key":p.command_key,"input_hash":p.input_hash,
        "expected_version":p.expected_version,"creator":p.creator,"request":p.request})
}
pub async fn queued_lock(
    tx: &mut Transaction<'_, Postgres>,
    h: &Queued,
    prepare: bool,
) -> Result<(), sqlx::Error> {
    let query = if prepare {
        sqlx::query("SELECT lock_prepare_queued_run_cleanup_wake119($1,$2,$3,$4,$5,$6)")
    } else {
        sqlx::query("SELECT lock_invocation119_cleanup_only($1,$2,$3,$4,$5,$6)")
    };
    query
        .bind(h.request.run_id().as_uuid())
        .bind(h.request.instance_id().as_uuid())
        .bind(original(h))
        .bind(h.fixture.scope.provider_namespace())
        .bind(h.fixture.scope.host_id())
        .bind(serde_json::to_vec(&h.fixture.config.canonical_json()).unwrap())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn cancel_lock(
    tx: &mut Transaction<'_, Postgres>,
    h: &Queued,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT lock_queued_cancel_delivery119($1,$2,$3,$4,$5)")
        .bind(h.request.run_id().as_uuid())
        .bind(serde_json::to_value(&h.cancel).unwrap())
        .bind(h.fixture.scope.provider_namespace())
        .bind(h.fixture.scope.host_id())
        .bind(serde_json::to_vec(&h.fixture.config.canonical_json()).unwrap())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn boundaries(pool: &PgPool, h: &Queued) {
    let before = rows(pool).await;
    for mode in 0..3 {
        for facade in 0..3 {
            let mut tx = h.fixture.birth.worker.begin().await.unwrap();
            let settings = match mode {
                0 => sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ WRITE"),
                1 => sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE READ WRITE"),
                _ => sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED READ ONLY"),
            };
            settings.execute(&mut *tx).await.unwrap();
            let error = if facade == 2 {
                cancel_lock(&mut tx, h).await.unwrap_err()
            } else {
                queued_lock(&mut tx, h, facade == 0).await.unwrap_err()
            };
            assert_eq!(code(&error).as_deref(), Some("P1180"));
            tx.rollback().await.unwrap();
            assert_eq!(rows(pool).await, before);
        }
    }
    for app in [true, false] {
        let mut tx = pool.begin().await.unwrap();
        let role = if app {
            sqlx::query("SET LOCAL ROLE hephaestus_app")
        } else {
            sqlx::query("SET LOCAL ROLE none")
        };
        role.execute(&mut *tx).await.unwrap();
        let error = queued_lock(&mut tx, h, true).await.unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("42501"));
        tx.rollback().await.unwrap();
        assert_eq!(rows(pool).await, before);
    }
    let public:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_proc p CROSS JOIN LATERAL aclexplode(p.proacl) acl WHERE p.proname IN ('discover_queued_run_cleanup_context119','load_invocation119_cleanup_only','load_queued_run_cleanup_candidate119','queued_run_cleanup_reservations119','lock_prepare_queued_run_cleanup_wake119','lock_invocation119_cleanup_only','queued_cancel_reservations119','lock_queued_cancel_delivery119','verify_queued_cancel_delivery119') AND acl.grantee=0 AND acl.privilege_type='EXECUTE')").fetch_one(pool).await.unwrap();
    assert!(
        !public,
        "PUBLIC is an ACL grantee, not the default owner caller"
    );
}
