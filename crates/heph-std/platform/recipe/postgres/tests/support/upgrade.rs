use std::borrow::Cow;

use recipe_postgres::PostgresDeploymentRepository;
use serde_json::Value;
use sqlx::{PgPool, migrate::Migrator};

use crate::{fixtures, seed, support};

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

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn historical_103_to_104_preserves_resources_and_admits_real_ledger(pool: PgPool) {
    checkpoint(103).run(&pool).await.expect("baseline 103");
    let highest: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("baseline marker");
    assert_eq!(
        highest,
        Some(103),
        "provider migration must be published before ledger upgrade checks"
    );
    let fixture = seed::seed(&pool).await;
    let before: Value = sqlx::query_scalar("SELECT jsonb_build_object('volume', (SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id = $1),
        'instance', (SELECT to_jsonb(instance) FROM agent_instances instance WHERE id = $2),
        'release', (SELECT to_jsonb(release) FROM releases release WHERE id = $3))")
        .bind(fixture.volume).bind(fixture.instance).bind(fixture.release).fetch_one(&pool).await.expect("historical snapshot");
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 103 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("historical checksums");
    checkpoint(104).run(&pool).await.expect("ledger migration");
    let after: Value = sqlx::query_scalar("SELECT jsonb_build_object('volume', (SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id = $1),
        'instance', (SELECT to_jsonb(instance) FROM agent_instances instance WHERE id = $2),
        'release', (SELECT to_jsonb(release) FROM releases release WHERE id = $3))")
        .bind(fixture.volume).bind(fixture.instance).bind(fixture.release).fetch_one(&pool).await.expect("preserved snapshot");
    assert_eq!(before, after);
    let after_checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 103 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("preserved checksums");
    assert_eq!(checksums, after_checksums);
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, true);
    let admitted = repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("real post-upgrade admission");
    assert_eq!(admitted.snapshot.intent, intent);
    eprintln!(
        "REAL_RECIPE_LEDGER_UPGRADE=1 historical_schema=103 checkpoint_schema=104 existing_resources_preserved=1 old_checksums_preserved=1 post_upgrade_admission=1 database_only=1"
    );
}
