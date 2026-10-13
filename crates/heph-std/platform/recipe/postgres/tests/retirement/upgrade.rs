use recipe_application::{DeploymentLifecycle, InstallProgress};
use serde_json::Value;
use sqlx::{PgPool, migrate::Migrator};

use crate::{harness::Harness, helpers};

fn checkpoint(version: i64) -> Migrator {
    crate::private_migrations::checkpoint(version)
}

async fn historical(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
        'definitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY to_jsonb(row)::text) FROM recipe_definitions row),
        'deployments',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployments row),
        'resources',(SELECT jsonb_agg(to_jsonb(row) ORDER BY resource_name) FROM recipe_deployment_resources row),
        'commands',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_commands row),
        'provenance',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_admission_attempts row),
        'attempts',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_attempts row),
        'proofs',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_verifications row),
        'transitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_execution_transitions row),
        'results',(SELECT jsonb_agg(to_jsonb(row) ORDER BY command_id) FROM recipe_command_results row),
        'volumes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_state_volumes row),
        'instances',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instances row),
        'leases',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_volume_leases row),
        'runs',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM runs row),
        'events',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM application_events row))")
        .fetch_one(pool).await.expect("historical ledger and actual resource rows")
}

#[sqlx::test(migrations = false)]
#[ignore = "requires ordered published 0108/0109 and disposable PostgreSQL through DATABASE_URL"]
async fn historical_108_to_109_preserves_pending_read_only_claim_then_retires_it_without_provider_proof(
    pool: PgPool,
) {
    checkpoint(108)
        .run(&pool)
        .await
        .expect("frozen baseline 108");
    let baseline: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("baseline marker");
    assert_eq!(
        baseline,
        Some(108),
        "runtime/import siblings must be published before retirement upgrade verification"
    );
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let retirement = helpers::admit_cleanup(&h, &cleanup, &target).await;
    let before = historical(&pool).await;
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=108 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("baseline checksums");
    checkpoint(109).run(&pool).await.expect("frozen target 109");
    let target_head: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("target marker");
    assert_eq!(target_head, Some(109));
    assert_eq!(before, historical(&pool).await);
    let after_checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=108 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("unchanged migration checksums");
    assert_eq!(checksums, after_checksums);
    let retired = h
        .repository
        .retire_external_observation(&cleanup, &retirement)
        .await
        .expect("actual post-upgrade retirement");
    assert_eq!(retired.install, InstallProgress::Creating);
    assert_eq!(retired.active_attempt, None);
    let snapshot = h
        .repository
        .inspect(&cleanup, h.intent.id())
        .await
        .expect("post-upgrade removal");
    let result = h
        .repository
        .finish_command(
            &cleanup,
            retirement.command,
            h.intent.id(),
            snapshot.version,
        )
        .await
        .expect("retained reference-only removal");
    assert_eq!(result.lifecycle, DeploymentLifecycle::Removed);
    let proofs: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_effect_verifications")
        .fetch_one(&pool)
        .await
        .expect("no provider receipts");
    assert_eq!(proofs, 0);
    eprintln!(
        "REAL_RECIPE_EXTERNAL_RETIREMENT_UPGRADE=1 historical_schema=108 checkpoint_schema=109 existing_rows_preserved=1 old_checksums_preserved=1 pending_claim_preserved=1 actual_retirement=1 provider_receipts=0 database_only=1"
    );
}
