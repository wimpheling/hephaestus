use super::super::{ResolvedUiPublication, persist_ui_publication};
use super::common::UI;
use agent_config::parse_repository_uis;
use agent_config::ui::gateway_resolution::ResolvedGatewayUis;
use agent_config::ui::static_resolution::ResolvedStaticUis;
use gateway_domain::RoutePath;
use release_domain::ui::UiRoutePath;
use release_domain::{BuildRequestId, ReleaseId};
use serial_test::serial;
use sqlx::{Postgres, Transaction, postgres::PgPoolOptions};
use std::{env, time::Duration};
use uuid::Uuid;

fn publication_with_config(config: agent_config::ui::RepositoryUisConfig) -> ResolvedUiPublication {
    ResolvedUiPublication {
        source_manifest_revision_id: Uuid::new_v4(),
        config,
        normalized_ui_hash: [0; 32],
        normalized_gateway_hash: None,
        static_uis: ResolvedStaticUis { uis: Vec::new() },
        gateway_uis: ResolvedGatewayUis { uis: Vec::new() },
    }
}

async fn assert_no_publication_rows(
    transaction: &mut Transaction<'_, Postgres>,
    release_id: ReleaseId,
) {
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
           FROM (
               SELECT 1 FROM release_ui_source_snapshots WHERE release_id = $1
               UNION ALL
               SELECT 1 FROM release_ui_descriptors WHERE release_id = $1
               UNION ALL
               SELECT 1 FROM release_ui_static_files WHERE release_id = $1
               UNION ALL
               SELECT 1 FROM release_ui_managed_services WHERE release_id = $1
               UNION ALL
               SELECT 1 FROM release_ui_api_bindings WHERE release_id = $1
           ) AS publication_rows",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .expect("count publication rows in rejected transaction");
    assert_eq!(rows, 0, "guard must write no publication rows");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable PostgreSQL and explicit real-publication guard marker"]
async fn persist_ui_publication_rejects_typed_route_guards_before_sql() {
    assert_eq!(
        env::var("REAL_RELEASE_UI_PUBLICATION_GUARD").as_deref(),
        Ok("1"),
        "run this real-PostgreSQL test with REAL_RELEASE_UI_PUBLICATION_GUARD=1"
    );
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("HEPHAESTUS_POSTGRES_TEST_URL is required for this real-PostgreSQL test");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply release migrations");

    let parsed = parse_repository_uis(UI.as_bytes())
        .config
        .expect("valid publication fixture");
    let assistant_index = parsed
        .uis
        .iter()
        .position(|ui| ui.key.as_str() == "assistant")
        .expect("assistant UI");
    assert_eq!(parsed.uis[assistant_index].route_base.as_str(), "assistant");
    assert_eq!(
        parsed.uis[assistant_index].apis[0].route.as_str(),
        "/service/api"
    );
    let mut reserved = parsed.clone();
    let docs_index = reserved
        .uis
        .iter()
        .position(|ui| ui.key.as_str() == "docs")
        .expect("docs UI");
    reserved.uis[docs_index].route_base = UiRoutePath::parse("_heph").expect("reserved route");
    assert!(!agent_config::validate_ui_route_collisions(&reserved).is_empty());
    let mut collision = parsed;
    collision.uis[assistant_index].apis[0].route =
        RoutePath::parse("/assistant").expect("collision route");
    assert!(!agent_config::validate_ui_route_collisions(&collision).is_empty());

    for publication in [
        publication_with_config(reserved),
        publication_with_config(collision),
    ] {
        let release_id = ReleaseId::new();
        let build_request_id = BuildRequestId::new();
        let mut transaction = pool.begin().await.expect("begin publication transaction");
        let error =
            persist_ui_publication(&mut transaction, release_id, build_request_id, &publication)
                .await
                .expect_err("typed route guard must reject before SQL");
        assert!(matches!(
            error,
            super::super::ReleaseServiceError::InvalidStoredData
        ));
        assert_no_publication_rows(&mut transaction, release_id).await;
        transaction
            .rollback()
            .await
            .expect("rollback rejected publication");
    }
    println!(
        "REAL_RELEASE_UI_PUBLICATION_GUARD_EXECUTED=1 cases=2 exact_error=invalid_stored_data same_transaction_zero_rows=1"
    );
    pool.close().await;
}
