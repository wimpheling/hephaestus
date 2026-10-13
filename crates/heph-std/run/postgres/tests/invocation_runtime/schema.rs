use sqlx::PgPool;

pub async fn upgrade(pool: &PgPool, target: i64) {
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut migrations = crate::preparation_schema::checkpoint(113);
    let mut all = migrations.iter().cloned().collect::<Vec<_>>();
    for (version, paths) in [
        (
            114,
            vec![include_str!(
                "../../../../../../migrations/0119_qualified_instance_activation.sql"
            )],
        ),
        (
            115,
            vec![include_str!(
                "../../../../../../migrations/0120_qualified_instance_invocation.sql"
            )],
        ),
        (
            116,
            vec![include_str!(
                "../../../../../../migrations/0121_owned_ready_retention.sql"
            )],
        ),
        (
            117,
            vec![include_str!(
                "../../../../../../migrations/0122_qualified_invocation_consumption.sql"
            )],
        ),
        (
            119,
            vec![include_str!(
                "../../../../../../migrations/0123_qualified_invocation_leases.sql"
            )],
        ),
    ] {
        if version <= target {
            let sql = paths
                .iter()
                .copied()
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .join("\n");
            all.push(sqlx::migrate::Migration::new(
                version,
                "private_invocation_consumption".into(),
                sqlx::migrate::MigrationType::Simple,
                sql.into(),
                false,
            ));
        }
    }
    migrations.migrations = all.into();
    migrations.run(pool).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<$1 ORDER BY version",
    )
    .bind(before.last().unwrap().0 + 1)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(after, before, "prior107/private-chain checksums stay exact");
}
