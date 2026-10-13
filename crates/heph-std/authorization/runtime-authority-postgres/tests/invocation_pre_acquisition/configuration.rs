use super::fixture::{Fixture, code};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn canonical_config_and_trusted_model_cannot_be_substituted(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let before = f.rows().await;
    for pointer in [
        "/vm/namespace",
        "/vm/host_id",
        "/volume/namespace",
        "/volume/host_id",
        "/volume/root",
        "/platform/version",
    ] {
        let mut cfg = f.birth.config.canonical_json();
        *cfg.pointer_mut(pointer).unwrap() = serde_json::json!("changed");
        assert!(
            !f.raw(
                &f.birth.birth.worker,
                &f.run,
                &serde_json::to_vec(&cfg).unwrap(),
                AUTHORIZATION_MODEL_VERSION
            )
            .await
            .unwrap()
        );
    }
    for bytes in [b"".as_slice(), b"{", b"null", b"\xff"] {
        assert!(
            !f.raw(
                &f.birth.birth.worker,
                &f.run,
                bytes,
                AUTHORIZATION_MODEL_VERSION
            )
            .await
            .unwrap()
        );
    }
    for model in [
        "",
        "client-selected",
        "qualified119/Unicode-λ",
        &"x".repeat(129),
    ] {
        assert!(
            f.repository
                .verify_qualified_invocation_pre_acquisition(&f.run, model)
                .await
                .is_err()
        );
        assert!(
            !f.raw(&f.birth.birth.worker, &f.run, &f.config(), model)
                .await
                .unwrap()
        );
    }
    let cfg = &f.birth.config;
    let changed = release_service::InstanceExecutionConfiguration::new(
        cfg.provider_scope().clone(),
        cfg.volume_root().to_owned(),
        cfg.volume_namespace(),
        cfg.platform_policy().clone(),
        "changed/server-platform".to_owned(),
    )
    .unwrap();
    let repository = PgRuntimeSessionRepository::new(f.birth.birth.worker.clone())
        .with_qualified_invocation_configuration(changed);
    assert!(
        repository
            .verify_qualified_invocation_pre_acquisition(&f.run, AUTHORIZATION_MODEL_VERSION)
            .await
            .is_err()
    );
    let error=sqlx::query("UPDATE agent_capability_bindings SET authorization_model_version='client-selected' WHERE instance_revision_id=$1")
        .bind(f.run.instance_revision_id.as_uuid()).execute(&pool).await.unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("23000"),
        "immutable binding exclusion, not a persisted model mismatch"
    );
    assert_eq!(f.rows().await, before);
    f.allowed().await;
}
