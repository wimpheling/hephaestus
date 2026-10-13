use super::fixture::Fixture;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use run_domain::RunState;
use run_orchestrator::{RunRepository, RunRuntimeCatalog};
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL and native ext4 tools"]
async fn source_withdrawal_preserves_full_own_set_but_denies_renewal_and_progress(pool: PgPool) {
    let h = Fixture::new(&pool, 2).await;
    let run = h.consume().await.unwrap().run;
    h.repository()
        .transition(run.id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h.volumes().load_run_selections(run.id).await.unwrap();
    let now = time::OffsetDateTime::now_utc();
    let leases = h
        .volumes()
        .acquire_run(
            &selected,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.birth.fixture.project)
        .bind(h.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = begin_actor_transaction(&h.birth.app, &h.birth.actor)
        .await
        .unwrap();
    let decision = PostgresMelangeAuthorizer
        .check(
            &mut tx,
            Subject::User(h.birth.actor.user_id),
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, h.birth.fixture.release_agent),
        )
        .await
        .unwrap();
    assert_eq!(decision, AuthorizationDecision::Deny);
    tx.rollback().await.unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert!(h.repository().load_runtime(&run).await.is_err());
    assert!(
        h.volumes()
            .heartbeat_run(
                &selected,
                h.scope.host_id(),
                now,
                now + time::Duration::minutes(6)
            )
            .await
            .is_err()
    );
    assert!(
        h.repository()
            .transition(run.id, RunState::Provisioning, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        h.volumes()
            .leases_for_run(run.id, h.scope.host_id())
            .await
            .unwrap(),
        leases,
        "held evidence remains readable without execution authority"
    );
    assert_eq!(
        h.repository()
            .inspect_consumed_invocation(run.id)
            .await
            .unwrap()
            .id,
        run.id
    );
    assert_eq!(h.rows().await, before);
    // Outcome/cleanup bookkeeping after withdrawal is independent of live
    // execution rights, but cannot release or clear an unknown physical fence.
    h.repository()
        .transition(run.id, RunState::Failed, None, Some("withdrawn"))
        .await
        .unwrap();
    h.repository()
        .transition(run.id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    let held = h.rows().await;
    assert!(
        h.repository()
            .transition(run.id, RunState::CleanedUp, None, None)
            .await
            .is_err()
    );
    assert_eq!(
        h.volumes()
            .leases_for_run(run.id, h.scope.host_id())
            .await
            .unwrap(),
        leases
    );
    assert_eq!(h.rows().await, held);
    h.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL"]
async fn default_options_wrong_configuration_and_wrong_planned_vm_deny_before_mutation(
    pool: PgPool,
) {
    let h = Fixture::new(&pool, 0).await;
    let before = h.rows().await;
    let ordinary = run_postgres::PgRunRepository::new(h.birth.worker.clone());
    assert!(
        ordinary
            .transition(h.start.run_id, RunState::LeasingVolume, None, None)
            .await
            .is_err()
    );
    assert!(
        ordinary
            .bind_resources(h.start.run_id, None, None, None, &h.vm.0)
            .await
            .is_err()
    );
    let default = volume_postgres::PostgresVolumeMetadataRepository::new(h.birth.worker.clone());
    assert!(default.load_run_selections(h.start.run_id).await.is_err());
    let wrong = release_service::InstanceExecutionConfiguration::new(
        vm_trait::VmProviderOwnerScope::new("wrong".to_owned(), h.scope.host_id().to_owned())
            .unwrap(),
        h.config.volume_root().to_owned(),
        h.config.volume_namespace(),
        h.config.platform_policy().clone(),
        h.config.platform_version().to_owned(),
    )
    .unwrap();
    assert!(
        h.volumes()
            .with_qualified_invocation_configuration(wrong)
            .load_run_selections(h.start.run_id)
            .await
            .is_err()
    );
    let vm = vm_trait::VmId("foreign".to_owned());
    assert!(
        h.repository()
            .create_run_with_vm_plan(&h.start, &h.scope, &vm)
            .await
            .is_err()
    );
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL and native ext4 tools"]
async fn admitted_remove_preserves_immutable_producer_and_denies_all_live_routes(pool: PgPool) {
    let h = Fixture::new(&pool, 2).await;
    let run = h.consume().await.unwrap().run;
    let manager = crate::support::identity(h.birth.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.birth.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let snapshot = h
        .birth
        .recipe
        .inspect(&manager, h.birth.intent.id())
        .await
        .unwrap();
    h.birth
        .recipe
        .admit_remove(
            &manager,
            crate::fixtures::remove(&manager, h.birth.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert!(h.volumes().load_run_selections(run.id).await.is_err());
    let immutable: bool =
        sqlx::query_scalar("SELECT run_profile_producer_matches(r) FROM runs r WHERE id=$1")
            .bind(run.id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(immutable);
    assert_eq!(
        h.repository()
            .inspect_consumed_invocation(run.id)
            .await
            .unwrap(),
        run
    );
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn actual_grant_revocation_denies_whole_set_without_clearing_live_fences(pool: PgPool) {
    use volume_trait::VolumeMountGrantRepository;
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let first = h
        .volumes()
        .acquire_run(
            &selected,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    let service = release_postgres::ReleaseService::new(
        h.birth.app.clone(),
        std::sync::Arc::new(PostgresMelangeAuthorizer),
    );
    service
        .revoke_volume_mount(
            &h.birth.actor,
            volume_domain::VolumeMountRevocationId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
            h.birth.input.volumes()[0].grant_id(),
        )
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.volumes().preflight_run(&selected).await.is_err());
    assert!(
        h.volumes()
            .acquire_run(
                &selected,
                h.scope.host_id(),
                now,
                now + time::Duration::minutes(5)
            )
            .await
            .is_err()
    );
    assert!(
        h.volumes()
            .heartbeat_run(
                &selected,
                h.scope.host_id(),
                now,
                now + time::Duration::minutes(6)
            )
            .await
            .is_err()
    );
    assert_eq!(
        h.volumes()
            .leases_for_run(h.start.run_id, h.scope.host_id())
            .await
            .unwrap(),
        first
    );
    assert_eq!(h.rows().await, before);
    h.physical.assert_unchanged();
}
