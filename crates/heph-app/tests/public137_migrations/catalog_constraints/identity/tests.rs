//! Genuine captures exercise schema ownership independently of helper ownership.
use super::super::super::{catalog, catalog_stability, fixture};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn public_namespace_owner_is_preserved_and_does_not_replace_routine_owner(pool: PgPool) {
    fixture::run138(&pool).await;
    let before = catalog::capture(&pool)
        .await
        .expect("genuine SQLx138 catalog");
    assert_eq!(
        super::public_namespace(&before)["owner"],
        "pg_database_owner"
    );
    assert_eq!(super::helpers(&before).len(), 2);
    catalog_stability::assert_untouched(&before, &before);
    catalog_stability::assert_replay(&before, &before);

    sqlx::query("ALTER SCHEMA public OWNER TO postgres")
        .execute(&pool)
        .await
        .expect("actual namespace-owner substitution");
    let changed = catalog::capture(&pool)
        .await
        .expect("changed namespace catalog");
    assert_eq!(super::public_namespace(&changed)["owner"], "postgres");
    assert_ne!(
        before.domains()["namespaces"],
        changed.domains()["namespaces"]
    );
    assert_eq!(super::helpers(&before), super::helpers(&changed));
    denied(
        || catalog_stability::assert_untouched(&before, &changed),
        &["PostgreSQL JSONB text changed: namespaces"],
    );

    sqlx::query(
        "ALTER FUNCTION public.private_volume_guest_path_is_valid(text) OWNER TO hephaestus_app",
    )
    .execute(&pool)
    .await
    .expect("actual helper-owner substitution");
    let changed_helper = catalog::capture(&pool)
        .await
        .expect("changed helper catalog");
    denied(
        || {
            let _bindings = super::helpers(&changed_helper);
        },
        &["hephaestus_app", "postgres"],
    );
    pool.close().await;
}

fn denied(verification: impl FnOnce() + std::panic::UnwindSafe, expected: &[&str]) {
    let failure = std::panic::catch_unwind(verification).expect_err("substitution must be denied");
    let message = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .expect("assertion panic text");
    for part in expected {
        assert!(
            message.contains(part),
            "wrong substitution denial: {message}"
        );
    }
}
