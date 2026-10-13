//! Restrictive revision-storage faults for the actual importer integration test.
//! Kept in the declared database test adapter; no production guard is disabled.

use sqlx::PgPool;

pub async fn fail_revisions(pool: &PgPool) {
    sqlx::query(include_str!(
        "../../../../../../../migrations/test_fixtures/recipe_import/reject_revision_function.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(include_str!(
        "../../../../../../../migrations/test_fixtures/recipe_import/reject_revision_trigger.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}

pub async fn allow_revisions(pool: &PgPool) {
    sqlx::query(include_str!(
        "../../../../../../../migrations/test_fixtures/recipe_import/allow_revision_trigger.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(include_str!(
        "../../../../../../../migrations/test_fixtures/recipe_import/allow_revision_function.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}
