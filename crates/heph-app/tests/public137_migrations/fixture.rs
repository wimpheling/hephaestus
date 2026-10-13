//! Trusted `SQLx` migration entrypoints; no manually manufactured journal rows.
use super::history::{self, History};
use sqlx::{PgConnection, PgPool, migrate::Migrator};
use std::borrow::Cow;

type MigrationContext = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub const MIGRATIONS: Migrator = sqlx::migrate!("../../migrations");

pub fn public107() -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 107)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

pub async fn run137(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 137)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    assert_eq!(
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        (1..=137).collect::<Vec<_>>()
    );
    run(pool, &migrations).await;
}

pub async fn run138(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 138)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    assert_eq!(
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        (1..=138).collect::<Vec<_>>()
    );
    run(pool, &migrations).await;
}

pub async fn run139(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 139)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    assert_eq!(
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        (1..=139).collect::<Vec<_>>()
    );
    run(pool, &migrations).await;
}

pub async fn run140(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 140)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    assert_eq!(
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        (1..=140).collect::<Vec<_>>()
    );
    run(pool, &migrations).await;
}

pub async fn run141(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|entry| entry.version <= 141)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    assert_eq!(
        migrations
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>(),
        (1..=141).collect::<Vec<_>>()
    );
    run(pool, &migrations).await;
}

pub async fn run142(pool: &PgPool) {
    let migrations = Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|entry| entry.version <= 142)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    let versions: Vec<_> = migrations.iter().map(|entry| entry.version).collect();
    assert_eq!(versions, (1..=142).collect::<Vec<_>>());
    run(pool, &migrations).await;
}

pub async fn run143(pool: &PgPool) {
    assert_eq!(
        MIGRATIONS
            .iter()
            .map(|entry| entry.version)
            .collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    run(pool, &MIGRATIONS).await;
}

pub async fn populated107(pool: &PgPool) -> History {
    run(pool, &public107()).await;
    history::seed(pool).await
}

pub async fn run(pool: &PgPool, migrations: &Migrator) {
    let mut connection = pool
        .acquire()
        .await
        .expect("exclusive migration connection");
    migration_context(&mut connection).await;
    migrations
        .run(&mut *connection)
        .await
        .expect("normal SQLx migration with default journal guards");
}

async fn migration_context(connection: &mut PgConnection) {
    let context: MigrationContext = sqlx::query_as(
        "SELECT session_user::text, current_user::text, current_setting('role'),
                NULLIF(current_setting('hephaestus.actor_id',true),''),
                NULLIF(current_setting('hephaestus.request_id',true),''),
                NULLIF(current_setting('hephaestus.subject_type',true),''),
                NULLIF(current_setting('hephaestus.occurrence_id',true),'')",
    )
    .fetch_one(connection)
    .await
    .expect("actual migration owner and empty event context");
    assert_eq!(
        context,
        (
            "postgres".into(),
            "postgres".into(),
            "none".into(),
            None,
            None,
            None,
            None,
        )
    );
}
