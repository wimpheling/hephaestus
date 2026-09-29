use super::*;

impl IsolatedGoldenDatabase {
    pub async fn create(parent_url: &str) -> Self {
        let database_name = format!("hephaestus_golden_{}", uuid::Uuid::new_v4().simple());
        let mut maintenance_url = Url::parse(parent_url).expect("parse golden PostgreSQL URL");
        maintenance_url.set_path("/postgres");
        let mut target_url = Url::parse(parent_url).expect("parse golden PostgreSQL URL");
        target_url.set_path(&format!("/{database_name}"));
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(maintenance_url.as_str())
            .await
            .expect("connect golden PostgreSQL maintenance database");
        let create_sql = format!("CREATE DATABASE \"{database_name}\"");
        sqlx::query(&create_sql)
            .execute(&maintenance_pool)
            .await
            .expect("create isolated golden database");
        maintenance_pool.close().await;

        let target_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(target_url.as_str())
            .await
            .expect("connect isolated golden database");
        sqlx::migrate!("../../migrations")
            .run(&target_pool)
            .await
            .expect("apply migrations to isolated golden database");
        target_pool.close().await;

        Self {
            database_name,
            maintenance_url: maintenance_url.to_string(),
            target_url: target_url.to_string(),
        }
    }

    pub async fn cleanup(self, pool: sqlx::PgPool) {
        pool.close().await;
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.maintenance_url)
            .await
            .expect("reconnect golden PostgreSQL maintenance database");
        let drop_sql = format!("DROP DATABASE \"{}\"", self.database_name);
        sqlx::query(&drop_sql)
            .execute(&maintenance_pool)
            .await
            .expect("drop isolated golden database after all pools closed");
        maintenance_pool.close().await;
    }
}

pub async fn product_outbox_census(database_url: &str) -> ProductOutboxCensus {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .expect("connect product outbox census database");
    let row = sqlx::query(
        "SELECT count(*)::bigint AS total,
                count(*) FILTER (
                    WHERE published_at IS NULL AND dead_lettered_at IS NULL
                )::bigint AS pending,
                count(*) FILTER (WHERE published_at IS NOT NULL)::bigint AS published,
                count(*) FILTER (WHERE dead_lettered_at IS NOT NULL)::bigint AS dead_lettered,
                md5(COALESCE(string_agg(
                    event_id::text, ',' ORDER BY event_id
                ) FILTER (
                    WHERE published_at IS NULL AND dead_lettered_at IS NULL
                ), '')) AS pending_fingerprint
           FROM product_event_outbox",
    )
    .fetch_one(&pool)
    .await
    .expect("read product outbox census");
    let census = ProductOutboxCensus {
        total: row.get("total"),
        pending: row.get("pending"),
        published: row.get("published"),
        dead_lettered: row.get("dead_lettered"),
        pending_fingerprint: row.get("pending_fingerprint"),
    };
    pool.close().await;
    census
}

pub async fn finish_isolated_golden(
    isolated: IsolatedGoldenDatabase,
    pool: sqlx::PgPool,
    target_url: &str,
    parent_url: &str,
    parent_before: Option<ProductOutboxCensus>,
) {
    let target_after = product_outbox_census(target_url).await;
    let parent_after = parent_before
        .as_ref()
        .map(|_| async { product_outbox_census(parent_url).await });
    isolated.cleanup(pool).await;
    assert_eq!(
        target_after.pending, 0,
        "isolated golden outbox must be quiescent before database cleanup"
    );
    if let Some(parent_before) = parent_before {
        let parent_after = parent_after.expect("parent census future").await;
        assert_eq!(
            parent_after, parent_before,
            "golden bootstrap must not mutate the inherited parent outbox"
        );
    }
}
