use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};

const MIGRATIONS: Migrator = sqlx::migrate!("../../migrations");

type Context = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub async fn install(pool: &PgPool) {
    assert_eq!(
        MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
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
    MIGRATIONS
        .run(&mut *connection)
        .await
        .expect("genuine complete SQLx143 journal");
}

pub async fn role_pool(pool: &PgPool, worker: bool) -> PgPool {
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
