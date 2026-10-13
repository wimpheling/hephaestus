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
            vec!["/var/tmp/0114_qualified_instance_activation.corrected.sql"],
        ),
        (115, vec!["/var/tmp/0115_qualified_instance_invocation.sql"]),
        (116, vec!["/var/tmp/0116_owned_ready_retention.sql"]),
        (
            117,
            vec!["/var/tmp/0117_qualified_invocation_consumption.nowait.sql"],
        ),
        (119, vec!["/var/tmp/0119_qualified_invocation_leases.sql"]),
    ] {
        if version <= target {
            let sql = paths
                .iter()
                .map(|path| std::fs::read_to_string(path).unwrap())
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
