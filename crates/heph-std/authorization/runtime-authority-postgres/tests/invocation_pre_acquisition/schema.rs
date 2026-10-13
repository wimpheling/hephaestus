//! Hash-frozen additive loader; the prerequisite Fixture supplies119-A.

use sha2::{Digest, Sha256, Sha384};
use sqlx::PgPool;

fn exact(sql: &str, expected: &str) {
    assert_eq!(format!("{:x}", Sha256::digest(sql.as_bytes())), expected);
}

pub fn verify_inputs() {
    for (path, sql) in [
        (
            "0119_qualified_invocation_leases.sql",
            include_str!("../../../../../../migrations/0123_qualified_invocation_leases.sql"),
        ),
        (
            "0119_qualified_invocation_authority.sql",
            include_str!("../../../../../../migrations/0124_qualified_invocation_authority.sql"),
        ),
        (
            "0119_qualified_invocation_empty_secret.sql",
            include_str!("../../../../../../migrations/0125_qualified_invocation_empty_secret.sql"),
        ),
        (
            "0119_qualified_invocation_pre_acquisition.sql",
            include_str!(
                "../../../../../../migrations/0126_qualified_invocation_pre_acquisition.sql"
            ),
        ),
    ] {
        assert_eq!(std::fs::read_to_string(source_path(path)).unwrap(), sql);
    }
    for (path, hash) in [
        (
            "0115_qualified_instance_invocation.sql",
            "57a1e5cdb68b7dfae3b3e7e09bea48ea894ec26dcfc41aa40b3c40648b0a9193",
        ),
        (
            "0117_qualified_invocation_consumption.nowait.sql",
            "4d6fb43f06737ec28b8a078aac896387ae9462b4601d9668e8f85a7ac92b0183",
        ),
    ] {
        exact(&std::fs::read_to_string(source_path(path)).unwrap(), hash);
    }
}

pub async fn apply(pool: &PgPool) {
    exact(
        include_str!("../../../../../../migrations/0123_qualified_invocation_leases.sql"),
        "1e8cfbdb81b9eede747442434ee96103e33ed18641d6ea1d35b50f9fd5eaf0f4",
    );
    let installed: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=119")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        installed,
        Sha384::digest(include_bytes!(
            "../../../../../../migrations/0123_qualified_invocation_leases.sql"
        ))
        .to_vec()
    );
    for (sql, hash) in [
        (
            include_str!("../../../../../../migrations/0124_qualified_invocation_authority.sql"),
            "0d116862074139fd0de88518e4952bb0a70fb5167259c55625e0c09752555bc9",
        ),
        (
            include_str!("../../../../../../migrations/0125_qualified_invocation_empty_secret.sql"),
            "ead5c2814cddf4c59045e1223b20da1e35bc41396231e13c4518deab6fd9acf3",
        ),
    ] {
        exact(sql, hash);
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }
    let before = catalog(pool).await;
    let sql =
        include_str!("../../../../../../migrations/0126_qualified_invocation_pre_acquisition.sql");
    exact(
        sql,
        "3f38c3ce7e384d42c04c2ce922820025d02025283ce579900a64b748d928743b",
    );
    sqlx::raw_sql(sql).execute(pool).await.unwrap();
    assert_eq!(
        catalog(pool).await,
        before,
        "existing function bodies and ACLs stay exact"
    );
}
async fn catalog(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(jsonb_build_object('oid',p.oid,'definition',pg_get_functiondef(p.oid),'acl',p.proacl) ORDER BY p.oid),'[]') FROM pg_proc p WHERE p.pronamespace='public'::regnamespace AND p.prokind='f' AND p.proname<>'verify_invocation119_pre_acquisition'")
        .fetch_one(pool).await.unwrap()
}

#[path = "schema/assets.rs"]
mod assets;
use assets::source_path;
