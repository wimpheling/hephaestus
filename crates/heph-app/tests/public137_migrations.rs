//! Genuine `SQLx` fresh143 and populated107 upgrade tests.
//! Upgrade checks preserve complete logical rows and original journal records.
//! Captured catalogs and actual sequences are exact during replay; independently
//! qualified fresh143 metadata supplies the semantic upgrade reference.
//! Physical observations retain their existing explicitly bounded coverage.

// Reuse immutable SecretPG seeds; broker/inspection helpers are not used here.
#[allow(dead_code, unused_imports)]
#[path = "../../heph-std/secret/postgres/tests/postgres/support/mod.rs"]
mod support;

mod public137_migrations {
    mod catalog;
    mod catalog_constraints;
    mod catalog_defaults;
    mod catalog_routines;
    mod catalog_stability;
    mod catalog_structure;
    mod catalog_table_columns;
    mod events;
    mod fixture;
    mod forward;
    mod harness;
    mod history;
    mod journal;
    mod new_fields;
    mod preservation;
    mod public139;
    mod reference;
    mod snapshots;

    use sqlx::PgPool;

    const REFERENCE_TEST_PATH: &str = concat!(
        module_path!(),
        "::populated_public107_upgrade_to143_and_same_database_replay::fresh143_reference"
    );

    #[sqlx::test(migrations = false)]
    async fn fresh_public143_install_and_same_database_replay(
        pool: PgPool,
    ) -> Result<(), sqlx::Error> {
        let result = harness::fresh(&pool).await;
        pool.close().await;
        result
    }

    #[sqlx::test(migrations = false)]
    async fn populated_public107_upgrade_to143_and_same_database_replay(
        pool: PgPool,
    ) -> Result<(), sqlx::Error> {
        let result = reference::populated(&pool, REFERENCE_TEST_PATH).await;
        pool.close().await;
        result
    }
}
