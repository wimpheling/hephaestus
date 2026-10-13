use super::{
    observations::{held, rows},
    wake_fixture::Wake,
};
use run_orchestrator::{RunCleanupWakeRequest, RunRepository};
use sqlx::{Execute, PgPool};
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn every_original_pin_and_foreign_run_instance_deny_without_effect(pool: PgPool) {
    let h = Wake::new(&pool, 2).await;
    h.prepare().await;
    let before = rows(&pool).await;
    let (controller, _, log) = super::core::controller(&h);
    for index in 0..10 {
        let mut original = h.request.original().clone();
        let mut run = h.request.run_id();
        let mut instance = h.request.instance_id();
        match index {
            0 => original.drain_attempt = Uuid::new_v4(),
            1 => original.drain_command = Uuid::new_v4(),
            2 => original.removal_id = Uuid::new_v4(),
            3 => original.command_key[0] ^= 1,
            4 => original.input_hash[0] ^= 1,
            5 => original.expected_version += 1,
            6 => original.creator = Uuid::new_v4(),
            7 => original.request = Uuid::new_v4(),
            8 => run = runtime_types::RunId::new(),
            _ => instance = runtime_types::AgentInstanceId::new(),
        }
        let wrong = RunCleanupWakeRequest::new(run, instance, original).unwrap();
        for error in [
            h.repository()
                .load_cleanup_wake(&wrong, &h.fixture.scope)
                .await
                .unwrap_err(),
            h.repository()
                .prepare_cleanup_wake(&wrong, &h.fixture.scope)
                .await
                .unwrap_err(),
        ] {
            if index == 8 {
                // An absent Run is rejected by the lookup before qualification.
                assert!(
                    matches!(error, run_orchestrator::RepositoryError::NotFound(id) if id == run),
                    "original pin {index}: {error:?}"
                );
            } else {
                held(&error);
            }
        }
        assert!(controller.clone().wake_cleanup(&wrong).await.is_err());
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(rows(&pool).await, before, "original pin {index}");
    }
    h.fixture.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn same_count_lease_binding_and_target_replacements_are_immutable(pool: PgPool) {
    let h = Wake::new(&pool, 2).await;
    let prepared = h.prepare().await;
    let before = rows(&pool).await;
    // Attempt real protected mutations; constraints/triggers remain enabled.
    // Rejection is immutable-row evidence, never fabricated accepted history.
    for query in protected_mutations() {
        let statement = query.sql();
        let mut tx = h.fixture.birth.worker.begin().await.unwrap();
        let error = query
            .bind(h.request.run_id().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                super::observations::code(&error).as_deref(),
                Some("23000" | "23503" | "23505" | "23514" | "42501")
            ),
            "{statement}: {error:?}"
        );
        tx.rollback().await.unwrap();
        assert_eq!(rows(&pool).await, before);
        assert_eq!(h.read().await, prepared);
    }
    h.fixture.physical.assert_unchanged();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn every_configured_owner_and_full_volume_config_mismatch_is_held(pool: PgPool) {
    let h = Wake::new(&pool, 2).await;
    h.prepare().await;
    let before = rows(&pool).await;
    for index in 0..5 {
        let scope = vm_trait::VmProviderOwnerScope::new(
            if index == 0 {
                "foreign".into()
            } else {
                h.fixture.scope.provider_namespace().into()
            },
            if index == 1 {
                "foreign".into()
            } else {
                h.fixture.scope.host_id().into()
            },
        )
        .unwrap();
        let config = release_service::InstanceExecutionConfiguration::new(
            scope,
            if index == 2 {
                "/different-root".into()
            } else {
                h.fixture.config.volume_root().into()
            },
            if index == 3 {
                volume_domain::VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap()
            } else {
                h.fixture.config.volume_namespace()
            },
            h.fixture.config.platform_policy().clone(),
            if index == 4 {
                "platform/different".into()
            } else {
                h.fixture.config.platform_version().into()
            },
        )
        .unwrap();
        let wrong = h
            .repository()
            .with_qualified_invocation_configuration(config);
        configured_guard(
            index,
            &wrong
                .load_cleanup_wake(&h.request, &h.fixture.scope)
                .await
                .unwrap_err(),
        );
        configured_guard(
            index,
            &wrong
                .prepare_cleanup_wake(&h.request, &h.fixture.scope)
                .await
                .unwrap_err(),
        );
        assert_eq!(rows(&pool).await, before);
    }
    let foreign =
        run_domain::RunCleanupHostId::new("foreign".into(), h.fixture.scope.host_id().into())
            .unwrap();
    assert!(
        h.repository()
            .load_cleanup_wake(&h.request, &foreign)
            .await
            .is_err()
    );
    assert_eq!(rows(&pool).await, before);
}

// Only this configuration substitution case admits the existing composition guard.
fn configured_guard(index: usize, error: &run_orchestrator::RepositoryError) {
    let expected = if index < 2 {
        "Invocation composition differs"
    } else {
        "cleanup wake history is held or contradictory"
    };
    assert!(
        matches!(error, run_orchestrator::RepositoryError::InvalidData(actual) if *actual == expected),
        "configuration {index}: {error:?}"
    );
}

fn protected_mutations()
-> [sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments>; 22] {
    [
        sqlx::query("UPDATE agent_instance_volume_leases SET id=gen_random_uuid() WHERE run_id=$1"),
        sqlx::query(
            "INSERT INTO agent_instance_volume_leases (id,volume_id,instance_id,run_id,update_id,host_id,fencing_token,state,acquired_at,heartbeat_at,expires_at,attached_at,recovering_at,released_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash) SELECT gen_random_uuid(),l.volume_id,l.instance_id,l.run_id,l.update_id,l.host_id,l.fencing_token,l.state,l.acquired_at,l.heartbeat_at,l.expires_at,l.attached_at,l.recovering_at,l.released_at,l.selection_origin,l.selected_revision_id,l.selected_release_id,l.selected_release_agent_id,l.selected_project_id,l.slot_key,l.access_mode,l.guest_path,l.slot_required,l.minimum_capacity_bytes,l.release_contract_hash FROM agent_instance_volume_leases l WHERE run_id=$1 LIMIT 1",
        ),
        sqlx::query("UPDATE runs SET vm_id='foreign' WHERE id=$1"),
        sqlx::query(
            "UPDATE runs SET vm_provider_namespace=NULL,vm_host_id=NULL,vm_id=NULL WHERE id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET volume_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET instance_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET selected_revision_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET selected_project_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET selected_release_agent_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET access_mode='read_only' WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET release_contract_hash=decode(repeat('00',32),'hex') WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET fencing_token=fencing_token+1 WHERE run_id=$1",
        ),
        sqlx::query("UPDATE agent_instance_volume_leases SET host_id='foreign' WHERE run_id=$1"),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET slot_key='substituted' WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET guest_path='/substituted' WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET selected_release_id=gen_random_uuid() WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET selection_origin='historical_unproven' WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET slot_required=NOT slot_required WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET minimum_capacity_bytes=minimum_capacity_bytes+1 WHERE run_id=$1",
        ),
        sqlx::query(
            "UPDATE runs SET volume_selection_snapshot=jsonb_set(volume_selection_snapshot,'{selections,0,declaration,guest_path}','\"/substituted\"') WHERE id=$1",
        ),
        sqlx::query("UPDATE run_cleanup_targets SET generation=generation+1 WHERE run_id=$1"),
        sqlx::query(
            "UPDATE run_cleanup_targets SET target_digest=decode(repeat('00',32),'hex') WHERE run_id=$1",
        ),
    ]
}
