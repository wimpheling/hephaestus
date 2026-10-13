use recipe_application::DeploymentError;
use sqlx::PgPool;

use crate::{fixture, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn published_source_plans_exact_policy_without_changing_v1_intent(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let actor = support::identity(seeded.maintainer);
    let request = fixture::request(&seeded, false);
    let planner = fixture::planner(&pool, 512).await;
    let plan = planner
        .plan(&actor, &request)
        .await
        .expect("authoritative plan");
    let expected = crate::fixtures::build(
        &seeded,
        &crate::fixtures::source(&seeded, false),
        request.id(),
        "sqlite",
        request.inputs(),
        &[crate::fixtures::catalog(&seeded)],
        false,
    );
    assert_eq!(plan.intent(), &expected);
    assert_eq!(plan.sources().len(), 1);
    let source = &plan.sources()[0];
    assert_eq!(source.selected_policy(), &fixture::policy(128));
    assert_eq!(source.platform().policy(), &fixture::policy(512));
    assert_eq!(source.platform().version(), "platform/planning-v1");
    assert_eq!(
        source.runtime_contract_hash(),
        release_domain::ContentHash::digest(&serde_json::to_vec(&fixture::contract()).unwrap())
    );
    assert!(source.image().as_str().contains("@sha256:"));
    assert_eq!(
        planner.plan(&actor, &request).await.unwrap().intent(),
        plan.intent()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_deployments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "planning has no ledger admission or effects");
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn incorrect_authored_contract_hash_is_rejected(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), false, true).await;
    assert!(matches!(
        fixture::planner(&pool, 512)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, false)
            )
            .await,
        Err(DeploymentError::InvalidPlanningInput)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn mutable_image_is_rejected_even_with_matching_contract_hash(pool: PgPool) {
    let mut contract = fixture::contract();
    contract["image_reference"] = serde_json::json!("registry.example/sqlite:latest");
    let seeded = fixture::setup(&pool, contract, true, true).await;
    assert!(matches!(
        fixture::planner(&pool, 512)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, false)
            )
            .await,
        Err(DeploymentError::InvalidPlanningInput)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn incomplete_published_policy_is_rejected(pool: PgPool) {
    let mut contract = fixture::contract();
    contract.as_object_mut().unwrap().remove("policy_ceiling");
    let seeded = fixture::setup(&pool, contract, true, true).await;
    assert!(matches!(
        fixture::planner(&pool, 512)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, false)
            )
            .await,
        Err(DeploymentError::InvalidPlanningInput)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn incompatible_platform_is_rejected_without_clamping(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    assert!(matches!(
        fixture::planner(&pool, 64)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, false)
            )
            .await,
        Err(DeploymentError::IncompatiblePolicy)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn draft_source_cannot_be_planned(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, false).await;
    // Only published releases have the usable_repository tuple. The required
    // source permission check therefore rejects the draft before reading facts.
    let result = fixture::planner(&pool, 512)
        .await
        .plan(
            &support::identity(seeded.maintainer),
            &fixture::request(&seeded, false),
        )
        .await;
    assert!(
        matches!(result, Err(DeploymentError::AuthorizationDenied)),
        "draft result: {result:?}"
    );
}
