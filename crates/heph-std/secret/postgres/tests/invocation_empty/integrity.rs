use super::{fixture::Fixture, ports};
use crate::authority_harness::{code, pg_time};
use heph_secret::SecretMountMetadata;
use run_domain::{RunKind, RunState};
use run_orchestrator::RunSecretManager;
use runtime_authority::RuntimeSessionRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, LeaseId,
    ReleaseAgentId, ReleaseId, RunId, VolumeId,
};
use secret_postgres::PostgresSecretMountMetadata;
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn exact_run_pins_and_config_cannot_be_replaced_and_actor_helper_is_denied(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let run = f.run().await;
    let before = f.rows().await;
    let error = f.raw(&f.authority.birth.birth.app, &run).await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    let mut variants = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut candidate = run.clone();
            candidate.$field = $value;
            variants.push(candidate);
        }};
    }
    changed!(id, RunId::new());
    changed!(instance_id, AgentInstanceId::from_uuid(Uuid::new_v4()));
    changed!(
        instance_revision_id,
        AgentInstanceRevisionId::from_uuid(Uuid::new_v4())
    );
    changed!(release_id, ReleaseId::from_uuid(Uuid::new_v4()));
    changed!(release_agent_id, ReleaseAgentId::from_uuid(Uuid::new_v4()));
    changed!(command_id, CommandId::new());
    changed!(requires_state, !run.requires_state);
    changed!(vm_id, Some("foreign-vm".to_owned()));
    changed!(state, RunState::Starting);
    changed!(
        attachment_id,
        Some(AgentAttachmentId::from_uuid(Uuid::new_v4()))
    );
    changed!(volume_id, Some(VolumeId::from_uuid(Uuid::new_v4())));
    changed!(lease_id, Some(LeaseId::from_uuid(Uuid::new_v4())));
    changed!(lease_fencing_token, Some(1));
    for candidate in variants {
        f.denied_run_unchanged(&candidate).await;
    }
    // Core's ordinary route deliberately remains unchanged. Wrong kind is a
    // dedicated verifier denial, not a claim about ordinary dispatch behavior.
    let mut ordinary_claim = run.clone();
    ordinary_claim.kind = RunKind::Normal;
    assert!(
        f.metadata
            .verify_qualified_invocation_empty_bindings(&ordinary_claim)
            .await
            .is_err()
    );
    for pointer in [
        "/vm/namespace",
        "/vm/host_id",
        "/volume/namespace",
        "/volume/host_id",
        "/volume/root",
        "/platform/version",
    ] {
        let mut config = f.authority.birth.config.canonical_json();
        *config.pointer_mut(pointer).unwrap() = serde_json::json!("changed");
        assert!(
            !f.raw_config(
                &f.authority.birth.birth.worker,
                &run,
                serde_json::to_vec(&config).unwrap()
            )
            .await
            .unwrap()
        );
    }
    let mut config = f.authority.birth.config.canonical_json();
    config["platform"]["policy"]["memory_mib"] = serde_json::json!(1);
    assert!(
        !f.raw_config(
            &f.authority.birth.birth.worker,
            &run,
            serde_json::to_vec(&config).unwrap()
        )
        .await
        .unwrap()
    );
    let config = &f.authority.birth.config;
    let drift = release_service::InstanceExecutionConfiguration::new(
        config.provider_scope().clone(),
        config.volume_root().to_owned(),
        config.volume_namespace(),
        config.platform_policy().clone(),
        "changed/server-platform".to_owned(),
    )
    .unwrap();
    let manager = ports::manager(
        PostgresSecretMountMetadata::new(f.authority.birth.birth.worker.clone())
            .with_qualified_invocation_configuration(drift),
    );
    assert!(manager.prepare(&run).await.is_err());
    assert!(manager.reauthorize(&run).await.is_err());
    assert_eq!(f.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn saved_revision_nonempty_malformed_mutations_are_denied_and_unbound_rows_cannot_qualify(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    for value in [
        serde_json::json!([Uuid::new_v4()]),
        serde_json::json!({"not":"array"}),
        serde_json::Value::Null,
    ] {
        let before = f.rows().await;
        let error =
            sqlx::query("UPDATE agent_instance_revisions SET secret_bindings=$1 WHERE id=$2")
                .bind(&value)
                .bind(f.authority.birth.start.instance_revision_id.as_uuid())
                .execute(&pool)
                .await
                .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("23000"));
        assert_eq!(f.rows().await, before);
        // This is an actual schema-valid unbound revision, not an admitted115
        // producer.115 already requires [] and its revision is immutable.
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,secret_bindings,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable,diagnostics,created_by) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,$2,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable,diagnostics,created_by FROM agent_instance_revisions WHERE instance_id=$3 ORDER BY created_at,id LIMIT 1")
            .bind(id).bind(&value).bind(f.authority.birth.birth.fixture.instance)
            .execute(&pool).await.unwrap();
        let mut substituted = f.run().await;
        substituted.instance_revision_id = AgentInstanceRevisionId::from_uuid(id);
        f.denied_run_unchanged(&substituted).await;
    }
    f.allowed_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn partial_ceiling_is_not_absence_and_protected_commit_preserves_saved_graph(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let h = &f.authority;
    let before = f.rows().await;
    let mut raw = h.birth.birth.worker.begin().await.unwrap();
    sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$1,$2,$3,$4,$5)")
        .bind(h.birth.start.run_id.as_uuid()).bind(h.birth.start.instance_id.as_uuid())
        .bind(h.birth.start.instance_revision_id.as_uuid()).bind(h.snapshot.authorization_model_version())
        .bind(h.snapshot.normalized_hash().as_bytes().as_slice()).execute(&mut *raw).await.unwrap();
    let run = f.run().await;
    let verified: bool = sqlx::query_scalar(
        "SELECT verify_invocation119_empty_secrets($1,$2,$3,$4,$5,$6,$7,$8,$9,'provisioning')",
    )
    .bind(run.id.as_uuid())
    .bind(serde_json::to_vec(&h.birth.config.canonical_json()).unwrap())
    .bind(run.instance_id.as_uuid())
    .bind(run.instance_revision_id.as_uuid())
    .bind(run.release_id.as_uuid())
    .bind(run.release_agent_id.as_uuid())
    .bind(run.command_id.as_uuid())
    .bind(run.requires_state)
    .bind(run.vm_id.as_deref())
    .fetch_one(&mut *raw)
    .await
    .unwrap();
    assert!(
        !verified,
        "incomplete ceiling in caller transaction cannot mean no ceiling"
    );
    assert_eq!(
        code(&raw.commit().await.unwrap_err()).as_deref(),
        Some("23000")
    );
    assert_eq!(f.rows().await, before);
    f.allowed_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn revoked_ceiling_cannot_be_ignored_for_empty_secrets_and_metadata_remains(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let stored = f.authority.issue().await;
    let revoked = f
        .authority
        .repository
        .revoke(stored.id, pg_time(), "empty-family regression")
        .await
        .unwrap();
    f.denied_unchanged().await;
    assert_eq!(
        f.authority.repository.find(stored.id).await.unwrap(),
        Some(revoked)
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn current_lease_cap_and_expired_ceiling_deny_empty_monitor_without_renewal(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 2)).await;
    let stored = f.authority.issue().await;
    let selections = f
        .authority
        .birth
        .volumes()
        .load_run_selections(f.authority.birth.start.run_id)
        .await
        .unwrap();
    let now = pg_time();
    f.authority
        .birth
        .volumes()
        .heartbeat_run(
            &selections,
            f.authority.birth.scope.host_id(),
            now,
            now + time::Duration::minutes(10),
        )
        .await
        .unwrap();
    f.denied_unchanged().await;
    assert_eq!(
        f.authority
            .repository
            .expire(stored.expires_at)
            .await
            .unwrap(),
        1
    );
    f.denied_unchanged().await;
    assert!(
        f.authority
            .repository
            .find(stored.id)
            .await
            .unwrap()
            .is_some()
    );
}
