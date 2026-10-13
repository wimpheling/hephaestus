use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use recipe_application::{DeploymentError, DeploymentExecutionPreparationRepository};
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::harness::{self, Graph, Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn invalid_last_source_leaves_no_graph_event_audit_or_effect(pool: PgPool) {
    let h = Harness::new(&pool, Graph::BadLastSource, 113).await;
    let before = full_state(&pool).await;
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::InvalidPlanningInput)
    ));
    assert_eq!(before, full_state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn deferred_graph_failure_rolls_back_fresh_event_outbox_and_all_linked_audits(pool: PgPool) {
    let h = Harness::new(&pool, Graph::TwoInstances, 113).await;
    let before = full_state(&pool).await;
    // Raw actor writer test reaches SQL completeness guards. This intentionally
    // bypasses the owning Rust codec, which independently rejects forged grants.
    let mut input = serde_json::to_value(h.input()).unwrap();
    input["instances"][1]["volumes"][0]["grant_id"] =
        input["instances"][0]["volumes"][0]["grant_id"].clone();
    let canonical = serde_json::to_vec(&input).unwrap();
    let children = input["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|child| serde_json::to_vec(child).unwrap())
        .collect::<Vec<_>>();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
        .bind(h.intent.id().as_uuid())
        .bind(h.command.id().as_uuid())
        .bind(canonical)
        .bind(children)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await
        .unwrap();
    let staged: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_deployment_execution_preparations),
          (SELECT count(*) FROM recipe_prepared_instance_imports),
          (SELECT count(*) FROM recipe_execution_preparation_audits)",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!((staged.0, staged.1), (1, 2));
    assert!(staged.2 >= 2);
    let error = tx.commit().await.unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    assert_eq!(
        before,
        full_state(&pool).await,
        "deferred rollback includes scope counters/outbox/audits"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn actor_raw_dml_and_worker_admission_are_rejected_before_writes(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    assert_protected_catalog(&pool).await;
    let before = harness::evidence(&pool).await;
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = sqlx::query(
        "INSERT INTO recipe_deployment_execution_preparations(deployment_id) VALUES($1)",
    )
    .bind(h.intent.id().as_uuid())
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error =
        sqlx::query("INSERT INTO recipe_prepared_instance_imports(deployment_id) VALUES($1)")
            .bind(h.intent.id().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error =
        sqlx::query("INSERT INTO recipe_execution_preparation_audits(deployment_id) VALUES($1)")
            .bind(h.intent.id().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let worker = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let input = h.input();
    let children = input
        .instances()
        .iter()
        .map(|i| serde_json::to_vec(i).unwrap())
        .collect::<Vec<_>>();
    let error = sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
        .bind(h.intent.id().as_uuid())
        .bind(h.command.id().as_uuid())
        .bind(input.canonical_bytes().unwrap())
        .bind(children)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&worker)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    worker.close().await;
    assert_eq!(before, harness::evidence(&pool).await);
}

async fn assert_protected_catalog(pool: &PgPool) {
    let protected: bool = sqlx::query_scalar(
        "SELECT function.prosecdef
          AND owner.rolname NOT IN ('hephaestus_app','hephaestus_worker')
          AND (owner.rolsuper OR owner.rolbypassrls)
          AND replace(array_to_string(function.proconfig,','),' ','') =
              'search_path=pg_catalog,public,pg_temp'
          AND has_function_privilege('hephaestus_app',function.oid,'EXECUTE')
          AND NOT has_function_privilege('hephaestus_worker',function.oid,'EXECUTE')
          AND NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(function.proacl,
              acldefault('f',function.proowner))) acl WHERE acl.grantee=0
              AND acl.privilege_type='EXECUTE')
          AND (SELECT count(*)=3 AND bool_and(table_row.relrowsecurity
              AND table_row.relforcerowsecurity AND table_row.relowner=function.proowner
              AND has_table_privilege('hephaestus_app',table_row.oid,'SELECT')
              AND NOT has_table_privilege('hephaestus_app',table_row.oid,'INSERT')
              AND NOT has_table_privilege('hephaestus_app',table_row.oid,'UPDATE')
              AND NOT has_table_privilege('hephaestus_app',table_row.oid,'DELETE')
              AND NOT has_table_privilege('hephaestus_worker',table_row.oid,'INSERT'))
            FROM pg_class table_row JOIN pg_namespace namespace ON namespace.oid=table_row.relnamespace
            WHERE namespace.nspname='public' AND table_row.relname IN
              ('recipe_deployment_execution_preparations','recipe_prepared_instance_imports',
               'recipe_execution_preparation_audits'))
         FROM pg_proc function JOIN pg_roles owner ON owner.oid=function.proowner
         WHERE function.oid='public.write_recipe_execution_preparation(uuid,uuid,bytea,bytea[],text)'::regprocedure",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        protected,
        "actual writer owner/search path/ACL and forced RLS remain protected"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn missing_platform_version_fails_closed_without_borrowing_historical_event(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let before = full_state(&pool).await;
    let mut input = serde_json::to_value(h.input()).unwrap();
    input["platform"].as_object_mut().unwrap().remove("version");
    for child in input["instances"].as_array_mut().unwrap() {
        child["source"]["platform"]
            .as_object_mut()
            .unwrap()
            .remove("version");
    }
    let children = input["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|child| serde_json::to_vec(child).unwrap())
        .collect::<Vec<_>>();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
        .bind(h.intent.id().as_uuid())
        .bind(h.command.id().as_uuid())
        .bind(serde_json::to_vec(&input).unwrap())
        .bind(children)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    tx.rollback().await.unwrap();
    assert_eq!(before, full_state(&pool).await);
}

pub async fn full_state(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'events',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM application_events row),
      'audits',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM authorization_audit_events row),
      'outbox',(SELECT jsonb_agg(to_jsonb(row) ORDER BY event_id) FROM product_event_outbox row),
      'scopes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY scope_kind,scope_id) FROM application_event_scopes row),
      'versions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY scope_kind,scope_id,aggregate_type,aggregate_id) FROM application_aggregate_versions row),
      'headers',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id) FROM recipe_deployment_execution_preparations row),
      'imports',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,ordinal) FROM recipe_prepared_instance_imports row),
      'links',(SELECT jsonb_agg(to_jsonb(row) ORDER BY audit_id) FROM recipe_execution_preparation_audits row),
      'effects',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_attempts row))")
      .fetch_one(pool).await.unwrap()
}
