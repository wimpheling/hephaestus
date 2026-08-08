//! Opt-in real-`PostgreSQL` coverage for capability audit evidence and RLS.

use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use capability_audit::{
    CapabilityAuditContext, CapabilityAuditCursor, CapabilityAuditError, CapabilityAuditPage,
    CapabilityAuditReason, CapabilityAuditRepository, CapabilityDecision, CapabilityUseOutcome,
    NewCapabilityAuditEvent,
};
use capability_audit_postgres::PostgresCapabilityAuditRepository;
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, RuntimeSessionId,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::RunId;
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn records_redacted_exact_evidence_and_authorizes_inspection() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed(&pool).await;
    let repository = PostgresCapabilityAuditRepository::new(pool.clone());
    let started_at = OffsetDateTime::now_utc();
    let context = CapabilityAuditContext {
        runtime_session_id: RuntimeSessionId::from_uuid(fixture.session_id),
        snapshot_id: AuthorizationSnapshotId::from_uuid(fixture.snapshot_id),
        binding_id: CapabilityBindingId::from_uuid(fixture.binding_id),
        operation: CapabilityOperation::Inspect,
        request_id: RequestId::new(),
        authorization_model_version: AUTHORIZATION_MODEL_VERSION,
    };
    repository
        .append(&NewCapabilityAuditEvent::decision(
            context,
            CapabilityDecision::Allow,
            None,
            started_at,
        ))
        .await
        .expect("record exact authorization decision");
    repository
        .append(&NewCapabilityAuditEvent::capability_use(
            context,
            CapabilityUseOutcome::Succeeded,
            Some(CapabilityAuditReason::parse("completed").expect("safe reason")),
            started_at + Duration::milliseconds(1),
        ))
        .await
        .expect("record exact capability use");

    let first_page = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(1, None).expect("bounded page"),
        )
        .await
        .expect("authorized audit inspection");
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].outcome, Some(CapabilityUseOutcome::Succeeded));
    assert_eq!(first_page[0].resource.id, fixture.repository_id);
    assert_eq!(first_page[0].slot.as_str(), "source");

    let second_page = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(
                10,
                Some(CapabilityAuditCursor {
                    occurred_at: first_page[0].occurred_at,
                    id: first_page[0].id,
                }),
            )
            .expect("cursor page"),
        )
        .await
        .expect("second audit page");
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].decision, Some(CapabilityDecision::Allow));

    let mut session_tx = begin_actor_transaction(&pool, &fixture.owner)
        .await
        .expect("authorized session inspection transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *session_tx)
        .await
        .expect("use non-bypass application role");
    let sessions: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT id, run_id, status
         FROM inspect_runtime_authority_sessions($1, 10)",
    )
    .bind(fixture.instance_id)
    .fetch_all(&mut *session_tx)
    .await
    .expect("inspect redacted runtime sessions");
    assert_eq!(
        sessions,
        vec![(fixture.session_id, fixture.run_id, String::from("active"))]
    );
    session_tx
        .commit()
        .await
        .expect("session inspection commit");

    assert_eq!(
        repository
            .list_for_run(
                &fixture.outsider,
                RunId::from_uuid(fixture.run_id),
                CapabilityAuditPage::new(10, None).expect("bounded page"),
            )
            .await,
        Err(CapabilityAuditError::Unavailable)
    );

    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name
         FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = 'capability_audit_inspection'
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect redacted view schema");
    for forbidden in ["credential", "payload", "path", "secret", "response"] {
        assert!(
            columns.iter().all(|column| !column.contains(forbidden)),
            "inspection schema exposed forbidden field family {forbidden}"
        );
    }
}

#[tokio::test]
#[serial]
async fn rejects_forged_ceiling_and_mutation() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed(&pool).await;
    let repository = PostgresCapabilityAuditRepository::new(pool.clone());
    let context = CapabilityAuditContext {
        runtime_session_id: RuntimeSessionId::from_uuid(fixture.session_id),
        snapshot_id: AuthorizationSnapshotId::from_uuid(fixture.snapshot_id),
        binding_id: CapabilityBindingId::from_uuid(fixture.binding_id),
        operation: CapabilityOperation::GitRead,
        request_id: RequestId::new(),
        authorization_model_version: AUTHORIZATION_MODEL_VERSION,
    };
    assert!(
        repository
            .append(&NewCapabilityAuditEvent::decision(
                context,
                CapabilityDecision::Allow,
                None,
                OffsetDateTime::now_utc(),
            ))
            .await
            .is_err(),
        "operation outside the immutable binding must fail closed"
    );

    let valid = NewCapabilityAuditEvent::decision(
        CapabilityAuditContext {
            operation: CapabilityOperation::Inspect,
            ..context
        },
        CapabilityDecision::Deny,
        Some(CapabilityAuditReason::parse("live_revoked").expect("safe reason")),
        OffsetDateTime::now_utc(),
    );
    repository
        .append(&valid)
        .await
        .expect("record valid denial");
    assert!(
        sqlx::query("UPDATE capability_audit_events SET reason_code = 'changed' WHERE id = $1")
            .bind(valid.id)
            .execute(&pool)
            .await
            .is_err(),
        "audit evidence must be immutable even for the table owner"
    );
    assert!(
        sqlx::query("DELETE FROM capability_audit_events WHERE id = $1")
            .bind(valid.id)
            .execute(&pool)
            .await
            .is_err(),
        "audit evidence must not be deletable"
    );
}

#[derive(Clone)]
struct Fixture {
    owner: AuthenticatedIdentity,
    outsider: AuthenticatedIdentity,
    repository_id: Uuid,
    instance_id: Uuid,
    run_id: Uuid,
    snapshot_id: Uuid,
    binding_id: Uuid,
    session_id: Uuid,
}

#[allow(clippy::too_many_lines)]
async fn seed(pool: &PgPool) -> Fixture {
    let owner = create_user(pool, "Capability audit owner").await;
    let outsider = create_user(pool, "Capability audit outsider").await;
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let requirement_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();

    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("audit-{organization_id}"))
        .execute(pool)
        .await
        .expect("audit organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("audit organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("audit-{project_id}"))
        .execute(pool)
        .await
        .expect("audit project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("audit-{repository_id}"))
        .execute(pool)
        .await
        .expect("audit repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'auditor')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("audit family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit build");
    sqlx::query(
        "INSERT INTO build_executions
         (build_request_id, vm_id, release_id, release_agent_id,
          release_version, state, exit_code)
         VALUES ($1, $2, $3, $4, 'v1', 'drafted', 0)",
    )
    .bind(build_id)
    .bind(format!("audit-vm-{build_id}"))
    .bind(release_id)
    .bind(release_agent_id)
    .execute(pool)
    .await
    .expect("audit build execution");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'draft')",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind(format!("audit-{release_id}"))
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'auditor', 'Auditor', $4, $5, false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(json!({"command": "bin/agent"}))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit release agent");
    sqlx::query(
        "INSERT INTO release_capability_requirements
         (id, release_agent_id, slot_key, purpose, resource_kind,
          required_operations, optional_operations, slot_required,
          normalized_hash)
         VALUES ($1, $2, 'source', 'Inspect source', 'repository',
                 ARRAY['inspect'], ARRAY[]::text[], true, $3)",
    )
    .bind(requirement_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit capability requirement");
    sqlx::query("UPDATE releases SET state = 'published', published_at = now() WHERE id = $1")
        .bind(release_id)
        .execute(pool)
        .await
        .expect("publish audit release");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state, created_by)
         VALUES ($1, $2, $3, $4, 'active', $5)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(format!("audit-{instance_id}"))
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("audit instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable, created_by)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'audit/v1', true, $6)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([6_u8; 32].as_slice())
    .bind([7_u8; 32].as_slice())
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("audit revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate audit revision");
    sqlx::query(
        "INSERT INTO agent_capability_bindings
         (id, instance_revision_id, release_agent_id, requirement_id,
          requirement_hash, slot_key, resource_kind, resource_id,
          granted_operations, normalized_hash, authorization_model_version,
          created_by)
         VALUES ($1, $2, $3, $4, $5, 'source', 'repository', $6,
                 ARRAY['inspect'], $7, $8, $9)",
    )
    .bind(binding_id)
    .bind(revision_id)
    .bind(release_agent_id)
    .bind(requirement_id)
    .bind([5_u8; 32].as_slice())
    .bind(repository_id)
    .bind([8_u8; 32].as_slice())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("audit binding");
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          run_kind, command_id, state, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'update', $6, 'running', false, now(), now())",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("audit run");
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
         (id, run_id, instance_id, instance_revision_id,
          authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(AUTHORIZATION_MODEL_VERSION)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit snapshot");
    sqlx::query(
        "INSERT INTO run_authorization_snapshot_bindings
         (snapshot_id, instance_revision_id, ordinal, binding_id,
          binding_hash, slot_key, resource_kind, resource_id,
          granted_operations)
         VALUES ($1, $2, 0, $3, $4, 'source', 'repository', $5,
                 ARRAY['inspect'])",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind([8_u8; 32].as_slice())
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("audit snapshot binding");
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          identity_hash, snapshot_hash, issuance_generation, credential_hash,
          status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, 'active',
                 now(), now() + interval '1 hour', now())",
    )
    .bind(session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind([10_u8; 32].as_slice())
    .bind([9_u8; 32].as_slice())
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("audit runtime session");

    Fixture {
        owner,
        outsider,
        repository_id,
        instance_id,
        run_id,
        snapshot_id,
        binding_id,
        session_id,
    }
}

async fn test_pool() -> Option<PgPool> {
    let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect to PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply capability audit migrations");
    Some(pool)
}

async fn create_user(pool: &PgPool, display_name: &str) -> AuthenticatedIdentity {
    let user_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.as_uuid())
        .bind(display_name)
        .execute(pool)
        .await
        .expect("capability audit user");
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("capability-audit-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
