//! Actual configured-role denial source plus private context shape controls.

use super::*;

#[test]
fn only_actual_worker_repeatable_read_readonly_context_is_accepted() {
    for (role, readonly, isolation) in [
        ("postgres", "on", "repeatable read"),
        ("hephaestus_app", "on", "repeatable read"),
        ("hephaestus_worker", "off", "repeatable read"),
        ("hephaestus_worker", "on", "read committed"),
    ] {
        assert_eq!(
            ensure_context(&(role.into(), readonly.into(), isolation.into())),
            Err(RuntimeAuthorityError::Unsupported)
        );
    }
    assert!(
        ensure_context(&(
            "hephaestus_worker".into(),
            "on".into(),
            "repeatable read".into()
        ))
        .is_ok()
    );
}

/// Requires C's already qualified ordinary public139 DB and real configured roles.
/// No migrations, grants, original/session writes, physical or positive graph proof.
#[tokio::test]
#[ignore = "requires qualified public139 admin URL with configured worker/application roles"]
async fn actual_roles_deny_admin_application_and_hold_missing_worker_original() {
    use run_domain::LegacyVmPlacementScope;
    use runtime_authority::RuntimeSessionRepository;
    use sqlx::postgres::PgPoolOptions;
    use std::env;
    use vm_trait::VmProviderOwnerScope;

    let url = env::var("HEPHAESTUS_EXPIRED_HANDOFF_TEST_ADMIN_URL")
        .expect("C-qualified ordinary public139 database URL");
    let scope = LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(
            Uuid::from_u128(91).to_string(),
            "role-denial-host".to_owned(),
        )
        .unwrap(),
    )
    .unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
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
        .connect(&url)
        .await
        .unwrap();
    let application = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let run = Uuid::new_v4();
    let session = RuntimeSessionId::from_uuid(Uuid::new_v4());
    let admin_repository = PgRuntimeSessionRepository::new(admin.clone())
        .with_expired_legacy_vm_placement_scope(scope.clone());
    let application_repository = PgRuntimeSessionRepository::new(application.clone())
        .with_expired_legacy_vm_placement_scope(scope.clone());
    let worker_repository = PgRuntimeSessionRepository::new(worker.clone())
        .with_expired_legacy_vm_placement_scope(scope);
    let admin_result = admin_repository
        .observe_expired_run_handoff(run, session, RuntimeCredentialGeneration::INITIAL)
        .await;
    let application_result = application_repository
        .observe_expired_run_handoff(run, session, RuntimeCredentialGeneration::INITIAL)
        .await;
    let worker_result = worker_repository
        .observe_expired_run_handoff(run, session, RuntimeCredentialGeneration::INITIAL)
        .await;
    drop(admin_repository);
    drop(application_repository);
    drop(worker_repository);
    worker.close().await;
    drop(worker);
    application.close().await;
    drop(application);
    admin.close().await;
    drop(admin);
    assert_eq!(
        admin_result.unwrap_err(),
        RuntimeAuthorityError::Unsupported
    );
    assert_eq!(
        application_result.unwrap_err(),
        RuntimeAuthorityError::Unsupported
    );
    assert_eq!(worker_result.unwrap_err(), RuntimeAuthorityError::NotFound);
}
