use std::borrow::Cow;

use recipe_application::{DiagnosticCode, ResourceAction};
use serde_json::Value;
use sqlx::{PgPool, migrate::Migrator};

use crate::harness::Harness;

static MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

fn checkpoint(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .migrations
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ..MIGRATIONS
    }
}

async fn historical(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
        'definitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY to_jsonb(row)::text) FROM recipe_definitions row),
        'deployments',(SELECT jsonb_agg(to_jsonb(row) ORDER BY to_jsonb(row)::text) FROM recipe_deployments row),
        'resources',(SELECT jsonb_agg(to_jsonb(row)-ARRAY['execution_generation','last_verified_action'] ORDER BY resource_name) FROM recipe_deployment_resources row),
        'commands',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_commands row),
        'provenance',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_admission_attempts row),
        'volumes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_state_volumes row),
        'instances',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instances row))")
        .fetch_one(pool).await.expect("historical immutable ledger and actual resource snapshot")
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn historical_105_to_106_preserves_admission_and_resources_then_claims_real_execution(
    pool: PgPool,
) {
    checkpoint(105)
        .run(&pool)
        .await
        .expect("frozen baseline 105");
    let baseline: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("baseline marker");
    assert_eq!(
        baseline,
        Some(105),
        "typed grants must be published before execution migration checks"
    );
    let h = Harness::new(&pool, true, false).await;
    let before = historical(&pool).await;
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=105 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("baseline checksums");
    checkpoint(106).run(&pool).await.expect("frozen target 106");
    let target: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("target marker");
    assert_eq!(target, Some(106));
    assert_eq!(before, historical(&pool).await);
    let after_checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=105 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("preserved checksums");
    assert_eq!(checksums, after_checksums);
    let initial: (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER (WHERE execution_generation=0 AND last_verified_action IS NULL) FROM recipe_deployment_resources")
        .fetch_one(&pool).await.expect("initial execution fences");
    assert_eq!(initial, (2, 2));
    let identity = h.request();
    let effect = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let claim = h
        .repository
        .claim_resource_effect(&identity, effect)
        .await
        .expect("real post-upgrade claim");
    h.repository
        .mark_resource_ambiguous(&identity, &claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .expect("durable blocked outcome");
    assert_eq!(claim.generation, 1);
    eprintln!(
        "REAL_RECIPE_EXECUTION_UPGRADE=1 historical_schema=105 checkpoint_schema=106 immutable_admission_preserved=1 resource_rows_preserved=1 old_checksums_preserved=1 real_claim_and_ambiguity=1 database_only=1"
    );
}
