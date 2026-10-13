use super::{
    observations::{held, rows},
    queued_fixture::Queued,
};
use run_orchestrator::{
    RepositoryError, RunCleanupRepository, RunCleanupWakeRequest, RunRepository,
};
use sqlx::PgPool;
use uuid::Uuid;

pub fn configuration(h: &Queued, index: usize) -> release_service::InstanceExecutionConfiguration {
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
    let mut policy = h.fixture.config.platform_policy().clone();
    if index == 5 {
        policy.vcpus += 1;
    }
    if index == 6 {
        policy.memory_mib += 1;
    }
    release_service::InstanceExecutionConfiguration::new(
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
        policy,
        if index == 4 {
            "platform/different".into()
        } else {
            h.fixture.config.platform_version().into()
        },
    )
    .unwrap()
}
fn config_error(index: usize, error: &RepositoryError) {
    let expected = if index < 2 {
        "Invocation composition differs"
    } else {
        "cleanup wake history is held or contradictory"
    };
    assert!(
        matches!(error,RepositoryError::InvalidData(value) if *value==expected),
        "{index}: {error:?}"
    );
}
#[sqlx::test(migrations = false)]
async fn every_original_full_configuration_scope_and_protected_target_pin_denies(pool: PgPool) {
    let h = Queued::new(&pool, 2, true).await;
    h.prepare().await;
    let before = rows(&pool).await;
    for index in 0..10 {
        let mut p = h.request.original().clone();
        let mut run = h.request.run_id();
        let mut instance = h.request.instance_id();
        match index {
            0 => p.drain_attempt = Uuid::new_v4(),
            1 => p.drain_command = Uuid::new_v4(),
            2 => p.removal_id = Uuid::new_v4(),
            3 => p.command_key[0] ^= 1,
            4 => p.input_hash[0] ^= 1,
            5 => p.expected_version += 1,
            6 => p.creator = Uuid::new_v4(),
            7 => p.request = Uuid::new_v4(),
            8 => run = runtime_types::RunId::new(),
            _ => instance = runtime_types::AgentInstanceId::new(),
        }
        let wrong = RunCleanupWakeRequest::new(run, instance, p).unwrap();
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
                assert!(matches!(error,RepositoryError::NotFound(id) if id==run));
            } else {
                held(&error);
            }
        }
        assert_eq!(rows(&pool).await, before, "original {index}");
    }
    for index in 0..7 {
        let config = configuration(&h, index);
        let wrong = h
            .repository()
            .with_qualified_invocation_configuration(config.clone());
        config_error(index, &wrong.get(h.request.run_id()).await.unwrap_err());
        config_error(
            index,
            &wrong
                .load_cleanup_wake(&h.request, &h.fixture.scope)
                .await
                .unwrap_err(),
        );
        config_error(
            index,
            &wrong
                .prepare_cleanup_wake(&h.request, &h.fixture.scope)
                .await
                .unwrap_err(),
        );
        let cleanup = run_postgres::PgRunCleanupRepository::new(
            h.fixture.birth.worker.clone(),
            h.fixture.scope.clone(),
        )
        .with_qualified_queued_invocation_cleanup(config);
        config_error(
            index,
            &cleanup.begin_cleanup(h.request.run_id()).await.unwrap_err(),
        );
        config_error(
            index,
            &cleanup
                .recorded_cleanup_receipt(h.request.run_id())
                .await
                .unwrap_err(),
        );
        assert_eq!(rows(&pool).await, before, "full configuration {index}");
    }
    let wrong =
        run_domain::RunCleanupHostId::new("foreign".into(), h.fixture.scope.host_id().into())
            .unwrap();
    assert!(matches!(
        h.repository().load_cleanup_wake(&h.request, &wrong).await,
        Err(RepositoryError::InvalidData(
            "owning cleanup wake is unsupported for this composition"
        ))
    ));
    for (query, expected) in [
        (
            sqlx::query("UPDATE runs SET vm_id='foreign' WHERE id=$1"),
            "23000",
        ),
        (
            sqlx::query("UPDATE runs SET vm_provider_namespace='foreign' WHERE id=$1"),
            "23000",
        ),
        (
            sqlx::query("UPDATE run_cleanup_targets SET generation=generation+1 WHERE run_id=$1"),
            "42501",
        ),
        (
            sqlx::query(
                "UPDATE run_cleanup_targets SET target_digest=decode(repeat('00',32),'hex') WHERE run_id=$1",
            ),
            "42501",
        ),
    ] {
        let mut tx = h.fixture.birth.worker.begin().await.unwrap();
        let error = query
            .bind(h.request.run_id().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(super::observations::code(&error).as_deref(), Some(expected));
        tx.rollback().await.unwrap();
        assert_eq!(rows(&pool).await, before);
    }
    h.fixture.physical.assert_unchanged();
}
