use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::borrow::Cow;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../migrations");

type Context = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(super) async fn install(pool: &PgPool) {
    // This fixture proves its historical ordinary139 contract, not the current head.
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
        migrations.iter().map(|m| m.version).collect::<Vec<_>>(),
        (1..=139).collect::<Vec<_>>()
    );
    let mut connection = pool.acquire().await.expect("exclusive migration owner");
    let actual: Context = sqlx::query_as(
        "SELECT session_user::text,current_user::text,current_setting('role'),
         NULLIF(current_setting('hephaestus.actor_id',true),''),
         NULLIF(current_setting('hephaestus.request_id',true),''),
         NULLIF(current_setting('hephaestus.subject_type',true),''),
         NULLIF(current_setting('hephaestus.occurrence_id',true),'')",
    )
    .fetch_one(&mut *connection)
    .await
    .expect("migration context");
    assert_eq!(
        actual,
        (
            "postgres".into(),
            "postgres".into(),
            "none".into(),
            None,
            None,
            None,
            None
        )
    );
    migrations
        .run(&mut *connection)
        .await
        .expect("genuine complete SQLx139 journal");
}

pub(super) async fn role_pool(pool: &PgPool, worker: bool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if worker {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                } else {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("actual dedicated role pool")
}
