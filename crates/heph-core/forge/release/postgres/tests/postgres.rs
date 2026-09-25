//! Opt-in real-PostgreSQL reusable release and isolated instance coverage.

use agent_config::{AgentConfig, parse, parse_repository_gateways, parse_repository_uis};
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::{
    CapabilityBindingId, CapabilityOperation, CapabilityResource, CapabilityResourceKind,
    CapabilitySlotKey,
};
use event_postgres::ReleaseOutboxPublisher;
use forge_domain::{GitRef, ProjectId, RepositoryId};
use futures_util::StreamExt;
use identity_domain::{
    AuthenticatedIdentity, OrganizationId, RequestId, UserId, actor_idempotency_id,
};
use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand, MailboxDispatchStore,
};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId,
    MailboxOperationIdentity, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ArtifactKind,
    ArtifactPath, BuildRequestId, ContentHash, InstanceName, NetworkAccess, ParameterName,
    ParameterValue, RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion, RuntimePolicy, TriggerPolicy, UiInstallationCallerKey,
    UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationOperation,
    UiInstallationTarget,
};
use release_postgres::{
    ActivateUiInstallation, BeginUpdateHook, CompleteBuild, CreateAttachment, CreateInstanceUpdate,
    DisableUiInstallation, ImportAgent, InstallStaticUi, InstallUi, RecoverInstanceUpdate,
    ReleaseArtifactInput, ReleaseService, RemoveAttachment, RemoveUiInstallation, ReviseInstance,
    ReviseInstanceCapabilities, RollbackUiInstallation, SetAttachmentEnabled, UiInstallationError,
    UpdateDecision, UpdateHookResult, UpdateRecoveryAction, UpdateRecoveryDecision,
};
use runtime_types::RunId;
use serde_json::{Value, json};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

type UiBindingRow = (String, String, Uuid, Uuid, Uuid, String, String, String);

struct Fixture {
    actor: UserId,
    organization: OrganizationId,
    first_project: ProjectId,
    first_repository: RepositoryId,
    first_aux_repository: RepositoryId,
    second_project: ProjectId,
    second_repository: RepositoryId,
    build: BuildRequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct BrokerRuleSnapshot {
    id: Uuid,
    binding_id: Uuid,
    instance_revision_id: Uuid,
    secret_version_id: Uuid,
    destination_origin: String,
    location_kind: String,
    header_name: String,
    header_prefix: Option<String>,
    normalized_hash: Vec<u8>,
}

#[path = "postgres/mod.rs"]
pub mod support;
use support::*;

#[path = "postgres/broker_rules.rs"]
pub mod broker_rules;

#[path = "postgres/complete_build.rs"]
pub mod complete_build;

#[path = "postgres/installation.rs"]
pub mod installation;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn install_ui_rejects_gateway_and_authority_mutations_without_receipt() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_id = publish_managed_release(&admin_pool, &worker_pool, &fixture).await;
    let (release_agent_id, release_agent_key): (Uuid, String) =
        sqlx::query_as("SELECT id, agent_key FROM release_agents WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("published release agent");
    let (gateway_id, revision_id) = seed_active_ui_gateway(
        &admin_pool,
        &fixture,
        release_id,
        release_agent_id,
        &release_agent_key,
        "http.service.v1",
        "heph_authenticated",
        "/service",
        &["GET"],
    )
    .await;
    let service = ReleaseService::new(worker_pool, Arc::new(PostgresMelangeAuthorizer));
    let (source_repository, source_project): (Uuid, Uuid) = sqlx::query_as(
        "SELECT repository.id, repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("read source project");

    let wrong_agent = seed_update_release(
        &admin_pool,
        release_id,
        ReleaseAgentId::from_uuid(release_agent_id),
    )
    .await;
    let wrong_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id = $1")
            .bind(wrong_agent.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("read wrong release");
    let mut case_number = 0_u32;
    let mut rejected = |label: &'static str| {
        case_number += 1;
        format!("general-negative-{case_number}-{label}")
    };

    let target = seed_same_org_target_project(&admin_pool, &fixture, "wrong-release").await;
    let wrong_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, $2, $3,
                release_agent_key, handler_contract, exposure, parameters,
                secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $5, created_by
         FROM gateway_revisions WHERE id = $4",
    )
    .bind(wrong_revision)
    .bind(wrong_release)
    .bind(wrong_agent.as_uuid())
    .bind(revision_id)
    .bind(vec![8_u8; 32])
    .execute(&admin_pool)
    .await
    .expect("seed wrong release revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         SELECT $1, $2, gateway_id, project_id, path, methods
         FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(wrong_revision)
    .bind(revision_id)
    .execute(&admin_pool)
    .await
    .expect("seed wrong release route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_revision)
        .execute(&admin_pool)
        .await
        .expect("activate wrong release revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("wrong-release"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore active release revision");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "wrong-agent").await;
    let same_release_wrong_agent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, release_id, family_id, agent_key || '-wrong', display_name,
                runtime_contract, $2, parameter_schema, secret_slot_schema,
                requires_state, update_hook
         FROM release_agents WHERE id = $3",
    )
    .bind(same_release_wrong_agent)
    .bind(vec![9_u8; 32])
    .bind(release_agent_id)
    .execute(&admin_pool)
    .await
    .expect("seed same-release wrong agent");
    let wrong_agent_revision = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_revisions
         (id, gateway_id, project_id, repository_id, release_id,
          release_agent_id, release_agent_key, handler_contract, exposure,
          parameters, secret_slots, mailbox_slots, service_loopback_port,
          service_readiness_path, service_health_path, service_log_capture_mode,
          normalized_hash, created_by)
         SELECT $1, gateway_id, project_id, repository_id, release_id, $2,
                release_agent_key, handler_contract, exposure, parameters,
                secret_slots, mailbox_slots, service_loopback_port,
                service_readiness_path, service_health_path, service_log_capture_mode,
                $3, created_by
         FROM gateway_revisions WHERE id = $4",
    )
    .bind(wrong_agent_revision)
    .bind(same_release_wrong_agent)
    .bind(vec![10_u8; 32])
    .bind(revision_id)
    .execute(&admin_pool)
    .await
    .expect("seed wrong agent revision");
    sqlx::query(
        "INSERT INTO gateway_routes
         (id, gateway_revision_id, gateway_id, project_id, path, methods)
         SELECT $1, $2, gateway_id, project_id, path, methods
         FROM gateway_routes WHERE gateway_revision_id = $3",
    )
    .bind(Uuid::new_v4())
    .bind(wrong_agent_revision)
    .bind(revision_id)
    .execute(&admin_pool)
    .await
    .expect("seed wrong agent route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_agent_revision)
        .execute(&admin_pool)
        .await
        .expect("activate wrong agent revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("wrong-agent"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore active agent revision");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "public").await;
    let public_revision = seed_gateway_revision_variant(
        &admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "public",
        "/service",
        &["GET"],
        true,
        11,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(public_revision)
        .execute(&admin_pool)
        .await
        .expect("activate public revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("public-exposure"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore gateway exposure");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "paused").await;
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(gateway_id)
        .execute(&admin_pool)
        .await
        .expect("pause gateway");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("paused"),
    )
    .await;
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(gateway_id)
        .execute(&admin_pool)
        .await
        .expect("restore gateway lifecycle");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "inactive-revision").await;
    sqlx::query("UPDATE gateways SET active_revision_id = NULL WHERE id = $1")
        .bind(gateway_id)
        .execute(&admin_pool)
        .await
        .expect("deactivate gateway revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("inactive-revision"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore active gateway revision");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "disabled-route").await;
    let disabled_route_revision = seed_gateway_revision_variant(
        &admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "heph_authenticated",
        "/service",
        &["GET"],
        false,
        12,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(disabled_route_revision)
        .execute(&admin_pool)
        .await
        .expect("activate disabled route revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("disabled-route"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore gateway route");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "wrong-method").await;
    let wrong_method_revision = seed_gateway_revision_variant(
        &admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "heph_authenticated",
        "/service",
        &["POST"],
        true,
        13,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_method_revision)
        .execute(&admin_pool)
        .await
        .expect("activate wrong method revision");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("wrong-method"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(&admin_pool)
        .await
        .expect("restore gateway method");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "source-read").await;
    // Keep release CanUse available through the repository's public-read
    // relation while removing every project-read path below.
    sqlx::query("UPDATE repositories SET is_public = true WHERE id = $1")
        .bind(source_repository)
        .execute(&admin_pool)
        .await
        .expect("enable source repository public read");
    // Organization membership grants project read transitively. Remove that
    // tuple as well as the direct source maintainer row so this case tests the
    // source-project barrier rather than an alternate organization path.
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source organization read");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source project read");
    assert_installation_denied_without_receipt(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("source-read"),
    )
    .await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("restore source project read");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("restore source organization read");
    sqlx::query("UPDATE repositories SET is_public = false WHERE id = $1")
        .bind(source_repository)
        .execute(&admin_pool)
        .await
        .expect("restore source repository visibility");

    let target = seed_same_org_target_project(&admin_pool, &fixture, "agent-use").await;
    // Release-agent use is derived from the release lifecycle. Revocation is
    // terminal, so exercise the real revoked state and its redacted category
    // as the final case in this disposable fixture.
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release_id.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke release for agent-use denial");
    assert_installation_denied_without_receipt_as(
        &service,
        &admin_pool,
        fixture.actor,
        target,
        release_id,
        rejected("agent-use"),
        UiInstallationError::PermissionDenied,
    )
    .await;
    println!("REAL_GENERAL_INSTALL_NEGATIVES=1 cases={case_number}");
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn install_ui_expected_organization_is_checked_before_replay() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "tenant-organization-guard",
    )
    .await;
    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: OrganizationId = OrganizationId::from_uuid(
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization"),
    );
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(foreign_organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote dual-member actor");

    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let wrong_tenant = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-wrong").expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(foreign_organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(matches!(
        wrong_tenant,
        Err(UiInstallationError::OrganizationMismatch)
    ));
    assert_no_installation(&admin_pool, fixture.first_project, "docs").await;

    let matching = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-match").expect("caller key"),
                target: UiInstallationTarget::project(fixture.first_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(fixture.organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("matching expected organization");
    assert_eq!(matching.state, release_domain::UiInstallationState::Enabled);

    let same_key = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-changed").expect("caller key"),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(fixture.organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await
        .expect("same organization command");
    let changed_tenant = service
        .install_ui(
            &identity(fixture.actor),
            InstallUi {
                caller_key: UiInstallationCallerKey::parse("tenant-changed").expect("caller key"),
                target: UiInstallationTarget::project(fixture.second_project),
                release_id,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                expected_organization_id: Some(foreign_organization),
                acknowledge_repository_git_access: false,
            },
        )
        .await;
    assert!(matches!(
        changed_tenant,
        Err(UiInstallationError::OrganizationMismatch)
    ));
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(same_key.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("same-key command count");
    assert_eq!(command_count, 1);
    let foreign_receipts: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM ui_installation_commands AS command
         JOIN ui_installations AS installation
           ON installation.id = command.installation_id
         WHERE installation.organization_id = $1
           AND command.caller_idempotency_key = 'tenant-changed'",
    )
    .bind(foreign_organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("foreign receipt absence");
    assert_eq!(foreign_receipts, 0);
}

// The fixture keeps each immutable gateway revision field explicit so every
// denial case visibly changes only its intended production property.
#[tokio::test]
#[serial]
// Keep the global authorization, tenancy, replay, and event cases in one
// real-PostgreSQL matrix so each assertion shares the same committed fixture.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_global_organization_matrix() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let worker_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&worker_pool)
        .await
        .expect("worker role identity");
    assert_eq!(worker_role, "hephaestus_worker");
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));

    let fixture = seed(&admin_pool).await;
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote global installation owner");
    let global_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "global",
        "matrix-global",
    )
    .await;

    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(global_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global release source project");
    let source_organization: Uuid = sqlx::query_scalar(
        "SELECT project.organization_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         JOIN projects AS project ON project.id = repository.project_id
         WHERE release.id = $1",
    )
    .bind(global_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global release source organization");
    assert_eq!(source_organization, fixture.organization.as_uuid());
    assert_ne!(source_project, fixture.first_project.as_uuid());

    let caller_key = UiInstallationCallerKey::parse("matrix-global-replay").expect("caller key");
    let first = service
        .install_static_ui(
            &identity(fixture.actor),
            InstallStaticUi {
                caller_key: caller_key.clone(),
                target: UiInstallationTarget::organization(fixture.organization),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("organization static UI install");
    assert_eq!(first.state, release_domain::UiInstallationState::Enabled);
    let owner_shape: (Uuid, Option<Uuid>, Option<Uuid>, String) = sqlx::query_as(
        "SELECT organization_id, project_id, repository_id, scope
         FROM ui_installations WHERE id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global installation owner shape");
    assert_eq!(
        owner_shape,
        (
            fixture.organization.as_uuid(),
            None,
            None,
            String::from("global")
        )
    );

    let replay = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("exact organization replay");
    assert_eq!(replay, first);
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'organization'
           AND event.aggregate_id = $2
           AND event.event_type = 'organization.changed'",
    )
    .bind(first.idempotency_id)
    .bind(fixture.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("organization owner event and product outbox");
    assert_eq!(durable_counts, (1, 1));

    let conflict = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "other",
            ),
        )
        .await;
    assert!(matches!(
        conflict,
        Err(UiInstallationError::IdempotencyConflict)
    ));

    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization");
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(foreign_organization)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote dual organization owner");
    let foreign_result = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-cross-org",
                UiInstallationTarget::organization(OrganizationId::from_uuid(foreign_organization)),
                global_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        foreign_result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    let foreign_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id = $1 AND scope = 'global' AND ui_key = 'docs'",
    )
    .bind(foreign_organization)
    .fetch_one(&admin_pool)
    .await
    .expect("foreign global installation absence");
    assert_eq!(foreign_count, 0);

    let second = seed(&admin_pool).await;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(second.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("seed second organization owner");
    let second_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &second,
        "global",
        "matrix-global-second",
    )
    .await;
    let second_source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(second_release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("second global release source project");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(second_source_project)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("seed second release use permission");
    let second_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-second-org",
                UiInstallationTarget::organization(second.organization),
                second_release,
                "docs",
            ),
        )
        .await
        .expect("same UI key in second organization");
    assert_eq!(
        second_install.state,
        release_domain::UiInstallationState::Enabled
    );
    let same_key_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id IN ($1, $2)
           AND scope = 'global' AND ui_key = 'docs' AND lifecycle <> 'removed'",
    )
    .bind(fixture.organization.as_uuid())
    .bind(second.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("same global key in two organizations");
    assert_eq!(same_key_count, 2);

    sqlx::query(
        "UPDATE organization_members
         SET role = 'member'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("revoke current global owner");
    let revoked_replay = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "matrix-global-replay",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await;
    assert!(matches!(
        revoked_replay,
        Err(UiInstallationError::PermissionDenied)
    ));
}

#[tokio::test]
#[serial]
// Every activation and rollback pins a fresh generation. This matrix also
// checks receipt replay after source revocation and rollback rollback safety.
#[allow(clippy::too_many_lines)]
async fn ui_installation_activation_rollback_generation_matrix() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-ui-generation-matrix").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_v1 = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "generation-v1",
    )
    .await;
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-install",
                UiInstallationTarget::project(fixture.first_project),
                release_v1,
                "docs",
            ),
        )
        .await
        .expect("install first generation");
    let generation_one_no: i64 =
        sqlx::query_scalar("SELECT generation_no FROM ui_installation_generations WHERE id = $1")
            .bind(installation.generation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("first generation number");
    assert_eq!(generation_one_no, 1);

    let activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate fresh generation");
    assert_ne!(activation.generation_id, installation.generation_id);
    assert_eq!(
        activation.state,
        release_domain::UiInstallationState::Enabled
    );
    let retained_generations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("retained generations after activation");
    assert_eq!(retained_generations, 2);
    let current_generation: Uuid =
        sqlx::query_scalar("SELECT current_generation_id FROM ui_installations WHERE id = $1")
            .bind(installation.installation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("current generation pointer");
    assert_eq!(current_generation, activation.generation_id.as_uuid());

    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(fixture.first_repository.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("seed repository owner for generation scope");
    let repository_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "repository",
        "generation-repository",
    )
    .await;
    let repository_installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-repository-install",
                UiInstallationTarget::repository(fixture.first_repository),
                repository_release,
                "docs",
            ),
        )
        .await
        .expect("install repository generation scope");
    let repository_activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-repository-activate")
                    .expect("repository activation key"),
                installation_id: repository_installation.installation_id,
                expected_generation_id: Some(repository_installation.generation_id),
                release_id: repository_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate repository generation scope");
    assert_ne!(
        repository_activation.generation_id,
        repository_installation.generation_id
    );
    let repository_rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-repository-rollback")
                    .expect("repository rollback key"),
                installation_id: repository_installation.installation_id,
                expected_generation_id: Some(repository_activation.generation_id),
                release_id: repository_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback repository generation scope");
    assert_ne!(
        repository_rollback.generation_id,
        repository_activation.generation_id
    );

    sqlx::query(
        "UPDATE organization_members SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote global owner for generation scope");
    let global_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "global",
        "generation-global",
    )
    .await;
    let global_installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "generation-global-install",
                UiInstallationTarget::organization(fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("install global generation scope");
    let global_activation = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-global-activate")
                    .expect("global activation key"),
                installation_id: global_installation.installation_id,
                expected_generation_id: Some(global_installation.generation_id),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activate global generation scope");
    assert_ne!(
        global_activation.generation_id,
        global_installation.generation_id
    );
    let global_rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-global-rollback")
                    .expect("global rollback key"),
                installation_id: global_installation.installation_id,
                expected_generation_id: Some(global_activation.generation_id),
                release_id: global_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback global generation scope");
    assert_ne!(
        global_rollback.generation_id,
        global_activation.generation_id
    );

    let activation_replay = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("exact activation replay");
    assert_eq!(activation_replay, activation);
    let release_v2 = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "generation-v2",
    )
    .await;
    let changed_input = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v2,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        changed_input,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let stale_cas = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-stale-cas").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        stale_cas,
        Err(UiInstallationError::GenerationConflict)
    ));

    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_v1.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("load source project for release revocation");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source release use");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source organization use");
    let revoked_replay = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("activation replay after source revocation");
    assert_eq!(revoked_replay, activation);
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(source_project)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("restore source release use");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member') ON CONFLICT DO NOTHING",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("restore source organization use");

    let same_input_new_caller = service
        .activate_ui_installation(
            &identity(fixture.actor),
            ActivateUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-activate-again")
                    .expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(activation.generation_id),
                release_id: release_v1,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("fresh same-input activation");
    assert_ne!(
        same_input_new_caller.generation_id,
        activation.generation_id
    );

    let rollback = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: UiInstallationCallerKey::parse("generation-rollback").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(same_input_new_caller.generation_id),
                release_id: release_v2,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await
        .expect("rollback to second release");
    assert_ne!(rollback.generation_id, same_input_new_caller.generation_id);
    assert_eq!(rollback.state, release_domain::UiInstallationState::Enabled);
    let generation_rows: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE id = $2)
         FROM ui_installation_generations WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .bind(installation.generation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("all immutable generations retained");
    assert_eq!(generation_rows, (4, 1));

    let invalid_release = publish_static_api_release(&admin_pool, &worker_pool, &fixture).await;
    let invalid_caller = UiInstallationCallerKey::parse("generation-invalid-binding").expect("key");
    let invalid_operation = UiInstallationCommandIdentity::new(
        fixture.actor.as_uuid(),
        UiInstallationOperation::Rollback,
        invalid_caller.clone(),
    );
    let invalid_occurrence = actor_idempotency_id(
        fixture.actor.as_uuid().as_bytes(),
        invalid_operation.command_key().as_bytes(),
    )
    .as_uuid();
    let before_invalid: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT\n             (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),\n             (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1),\n             (SELECT count(*) FROM application_events WHERE occurrence_id = $2),\n             (SELECT count(outbox.event_id) FROM application_events AS event\n              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id\n              WHERE event.occurrence_id = $2)",
    )
    .bind(installation.installation_id.as_uuid())
    .bind(invalid_occurrence)
    .fetch_one(&admin_pool)
    .await
    .expect("invalid rollback baseline");
    let invalid = service
        .rollback_ui_installation(
            &identity(fixture.actor),
            RollbackUiInstallation {
                caller_key: invalid_caller,
                installation_id: installation.installation_id,
                expected_generation_id: Some(rollback.generation_id),
                release_id: invalid_release,
                ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
            },
        )
        .await;
    assert!(matches!(
        invalid,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));
    let after_invalid: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT\n             (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),\n             (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1),\n             (SELECT count(*) FROM application_events WHERE occurrence_id = $2),\n             (SELECT count(outbox.event_id) FROM application_events AS event\n              LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id\n              WHERE event.occurrence_id = $2)",
    )
    .bind(installation.installation_id.as_uuid())
    .bind(invalid_occurrence)
    .fetch_one(&admin_pool)
    .await
    .expect("invalid rollback absence");
    assert_eq!(after_invalid, before_invalid);
    println!(
        "REAL_UI_INSTALLATION_GENERATION_MATRIX=1 generations=4 owner_scopes=project_repository_global replay=1 source_revocation=1 same_input_new_caller=1 stale_cas=1 invalid_binding_no_rows=1"
    );
}

#[tokio::test]
#[serial]
// Disable/remove use only current target management. The matrix deliberately
// revokes source-release use before replaying an exact disable command.
// This exception keeps all owner shapes and post-commit assertions in one
// matrix so replay and terminal-state evidence share one installation.
#[allow(clippy::too_many_lines)]
async fn ui_installation_disable_remove_lifecycle_matrix() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool().await else {
        return;
    };
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));

    let fixture = seed(&admin_pool).await;
    let release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "lifecycle-project",
    )
    .await;
    let installation = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-install",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("install lifecycle project fixture");
    let disabled = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("disable project installation");
    assert_eq!(
        disabled.state,
        release_domain::UiInstallationState::Disabled
    );
    assert_eq!(disabled.generation_id, installation.generation_id);
    let stored_disable_request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM ui_installation_commands
         WHERE installation_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-disable'",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("stored disable request provenance");

    let replay = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("exact disable replay");
    assert_eq!(replay, disabled);
    let replayed_disable_request: Uuid = sqlx::query_scalar(
        "SELECT request_id FROM ui_installation_commands
         WHERE installation_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-disable'",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("replayed disable request provenance");
    assert_eq!(replayed_disable_request, stored_disable_request);
    let conflict = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: None,
            },
        )
        .await;
    assert!(matches!(
        conflict,
        Err(UiInstallationError::IdempotencyConflict)
    ));
    let stale = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-stale").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(UiInstallationGenerationId::new()),
            },
        )
        .await;
    assert!(matches!(
        stale,
        Err(UiInstallationError::GenerationConflict)
    ));

    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("source project");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("revoke source release use");
    let revoked_replay = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("replay without source permission");
    assert_eq!(revoked_replay, disabled);

    let same_state = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-disable-again").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("fresh same-state disable");
    assert_eq!(same_state.generation_id, installation.generation_id);
    let removed = service
        .remove_ui_installation(
            &identity(fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-remove").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("remove project installation");
    assert_eq!(removed.state, release_domain::UiInstallationState::Removed);
    assert_eq!(removed.generation_id, installation.generation_id);
    let remove_replay = service
        .remove_ui_installation(
            &identity(fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-remove").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await
        .expect("exact remove replay");
    assert_eq!(remove_replay, removed);
    let terminal = service
        .disable_ui_installation(
            &identity(fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-terminal").expect("key"),
                installation_id: installation.installation_id,
                expected_generation_id: Some(installation.generation_id),
            },
        )
        .await;
    assert!(matches!(
        terminal,
        Err(UiInstallationError::InvalidTransition)
    ));
    let project_event_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.aggregate_type = 'project' AND event.aggregate_id = $1
           AND event.event_type = 'project.changed'
           AND event.request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $2
           )",
    )
    .bind(fixture.first_project.as_uuid())
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("project lifecycle event counts");
    assert_eq!(project_event_counts, (4, 4));
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(installation.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("project lifecycle command count");
    assert_eq!(command_count, 4);

    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(source_project)
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("restore source release use");
    let reused = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-reused-key",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("reuse removed installation key");
    assert_ne!(reused.installation_id, installation.installation_id);

    let repository_fixture = seed(&admin_pool).await;
    sqlx::query("INSERT INTO repository_managers (repository_id, user_id) VALUES ($1, $2)")
        .bind(repository_fixture.first_repository.as_uuid())
        .bind(repository_fixture.actor.as_uuid())
        .execute(&admin_pool)
        .await
        .expect("repository manager");
    let repository_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &repository_fixture,
        "repository",
        "lifecycle-repository",
    )
    .await;
    let repository_install = service
        .install_static_ui(
            &identity(repository_fixture.actor),
            install_command(
                "lifecycle-repository-install",
                UiInstallationTarget::repository(repository_fixture.first_repository),
                repository_release,
                "docs",
            ),
        )
        .await
        .expect("install repository lifecycle fixture");
    service
        .remove_ui_installation(
            &identity(repository_fixture.actor),
            RemoveUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-repository-remove")
                    .expect("key"),
                installation_id: repository_install.installation_id,
                expected_generation_id: Some(repository_install.generation_id),
            },
        )
        .await
        .expect("remove repository installation");
    let repository_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'repository' AND aggregate_id = $1
           AND event_type = 'repository.changed' AND related_id_one = $2
           AND request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $3
           )",
    )
    .bind(repository_fixture.first_repository.as_uuid())
    .bind(repository_fixture.first_project.as_uuid())
    .bind(repository_install.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("repository owner event");
    assert_eq!(repository_event, 2);

    let global_fixture = seed(&admin_pool).await;
    sqlx::query(
        "UPDATE organization_members SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(global_fixture.organization.as_uuid())
    .bind(global_fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("global owner");
    let global_release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &global_fixture,
        "global",
        "lifecycle-global",
    )
    .await;
    let global_install = service
        .install_static_ui(
            &identity(global_fixture.actor),
            install_command(
                "lifecycle-global-install",
                UiInstallationTarget::organization(global_fixture.organization),
                global_release,
                "docs",
            ),
        )
        .await
        .expect("install global lifecycle fixture");
    service
        .disable_ui_installation(
            &identity(global_fixture.actor),
            DisableUiInstallation {
                caller_key: UiInstallationCallerKey::parse("lifecycle-global-disable")
                    .expect("key"),
                installation_id: global_install.installation_id,
                expected_generation_id: Some(global_install.generation_id),
            },
        )
        .await
        .expect("disable global installation");
    let global_event: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'organization' AND aggregate_id = $1
           AND event_type = 'organization.changed'
           AND request_id IN (
               SELECT request_id FROM ui_installation_commands
               WHERE installation_id = $2
           )",
    )
    .bind(global_fixture.organization.as_uuid())
    .bind(global_install.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("global owner event");
    assert_eq!(global_event, 2);
}

#[tokio::test]
#[serial]
// Two distinct owner rows let both lifecycle transactions pass owner
// authorization independently while contending on the actor command ledger.
// The loser must retry its rolled-back mutation and report changed input.
#[allow(clippy::too_many_lines)]
async fn ui_installation_lifecycle_cross_owner_ledger_race_conflicts() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-ui-lifecycle-ledger-race").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "lifecycle-ledger-race",
    )
    .await;
    let service = ReleaseService::new(worker_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let first_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-ledger-race-first-install",
                UiInstallationTarget::project(fixture.first_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("first race installation");
    let second_install = service
        .install_static_ui(
            &identity(fixture.actor),
            install_command(
                "lifecycle-ledger-race-second-install",
                UiInstallationTarget::project(fixture.second_project),
                release,
                "docs",
            ),
        )
        .await
        .expect("second race installation");

    let mut first_owner_lock = admin_pool.begin().await.expect("begin first owner lock");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
        .bind(fixture.first_project.as_uuid())
        .fetch_one(&mut *first_owner_lock)
        .await
        .expect("hold first project owner lock");
    let mut second_owner_lock = admin_pool.begin().await.expect("begin second owner lock");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR UPDATE")
        .bind(fixture.second_project.as_uuid())
        .fetch_one(&mut *second_owner_lock)
        .await
        .expect("hold second project owner lock");

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_pool = worker_pool.clone();
    let first_id = first_install.installation_id;
    let first_generation = first_install.generation_id;
    let actor = fixture.actor;
    let first_task = tokio::spawn(async move {
        first_start.wait().await;
        ReleaseService::new(first_pool, Arc::new(PostgresMelangeAuthorizer))
            .disable_ui_installation(
                &identity(actor),
                DisableUiInstallation {
                    caller_key: UiInstallationCallerKey::parse("lifecycle-ledger-race")
                        .expect("race caller key"),
                    installation_id: first_id,
                    expected_generation_id: Some(first_generation),
                },
            )
            .await
    });
    let second_start = Arc::clone(&start);
    let second_pool = worker_pool.clone();
    let second_id = second_install.installation_id;
    let second_generation = second_install.generation_id;
    let second_actor = fixture.actor;
    let second_task = tokio::spawn(async move {
        second_start.wait().await;
        ReleaseService::new(second_pool, Arc::new(PostgresMelangeAuthorizer))
            .disable_ui_installation(
                &identity(second_actor),
                DisableUiInstallation {
                    caller_key: UiInstallationCallerKey::parse("lifecycle-ledger-race")
                        .expect("race caller key"),
                    installation_id: second_id,
                    expected_generation_id: Some(second_generation),
                },
            )
            .await
    });
    start.wait().await;
    wait_for_named_lock_waiters(&admin_pool, "heph-ui-lifecycle-ledger-race", 2).await;
    let (first_release, second_release) =
        tokio::join!(first_owner_lock.commit(), second_owner_lock.commit());
    first_release.expect("release first owner lock");
    second_release.expect("release second owner lock");

    let first_result = first_task.await.expect("first lifecycle race task");
    let second_result = second_task.await.expect("second lifecycle race task");
    let winner = match (first_result, second_result) {
        (Err(UiInstallationError::IdempotencyConflict), Ok(result))
        | (Ok(result), Err(UiInstallationError::IdempotencyConflict)) => result,
        (first, second) => {
            panic!("unexpected cross-owner ledger race results: {first:?}, {second:?}")
        }
    };
    let loser_id = if winner.installation_id == first_install.installation_id {
        second_install.installation_id
    } else {
        first_install.installation_id
    };
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE actor_id = $1 AND operation = 'disable'
           AND caller_idempotency_key = 'lifecycle-ledger-race'",
    )
    .bind(fixture.actor.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one winning lifecycle command");
    assert_eq!(command_count, 1);
    let event_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'project'
           AND event.aggregate_id IN ($2, $3)
           AND event.event_type = 'project.changed'",
    )
    .bind(winner.idempotency_id)
    .bind(fixture.first_project.as_uuid())
    .bind(fixture.second_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one winning lifecycle event and outbox");
    assert_eq!(event_counts, (1, 1));
    let winner_state: String =
        sqlx::query_scalar("SELECT lifecycle FROM ui_installations WHERE id = $1")
            .bind(winner.installation_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("winning installation state");
    let loser_state: String =
        sqlx::query_scalar("SELECT lifecycle FROM ui_installations WHERE id = $1")
            .bind(loser_id.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("losing installation state");
    assert_eq!(winner_state, "disabled");
    assert_eq!(loser_state, "enabled");
    println!(
        "REAL_UI_LIFECYCLE_LEDGER_RACE=1 winner={} loser={} command_rows={} event_rows={} outbox_rows={}",
        winner.installation_id, loser_id, command_count, event_counts.0, event_counts.1
    );
}

#[tokio::test]
#[serial]
// The owner-row lock is the production synchronization point: two real
// worker connections are held behind it, then released to race the same
// actor-bound command and prove one committed result is replayed exactly.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_concurrent_exact_replay_has_one_commit() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-replay").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "project",
        "matrix-concurrent-replay",
    )
    .await;
    let caller_key =
        UiInstallationCallerKey::parse("matrix-concurrent-replay").expect("caller key");
    let target = UiInstallationTarget::project(fixture.first_project);
    let command = install_command(caller_key.as_str(), target, release_id, "docs");

    let mut owner_lock = admin_pool.begin().await.expect("begin owner lock barrier");
    let owner_backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *owner_lock)
        .await
        .expect("read owner lock backend pid");
    sqlx::query("SELECT id FROM projects WHERE id = $1 FOR NO KEY UPDATE")
        .bind(fixture.first_project.as_uuid())
        .fetch_one(&mut *owner_lock)
        .await
        .expect("hold project owner lock");

    let start = Arc::new(tokio::sync::Barrier::new(3));
    let first_start = Arc::clone(&start);
    let first_pool = worker_pool.clone();
    let first_command = command.clone();
    let first_actor = fixture.actor;
    let first_task = tokio::spawn(async move {
        first_start.wait().await;
        ReleaseService::new(first_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(&identity(first_actor), first_command)
            .await
    });
    let second_start = Arc::clone(&start);
    let second_pool = worker_pool.clone();
    let second_command = command;
    let second_actor = fixture.actor;
    let second_task = tokio::spawn(async move {
        second_start.wait().await;
        ReleaseService::new(second_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(&identity(second_actor), second_command)
            .await
    });
    start.wait().await;
    wait_for_row_lock_waiters(
        &admin_pool,
        "projects",
        owner_backend_pid,
        "heph-static-install-replay",
        2,
        false,
    )
    .await;
    owner_lock
        .commit()
        .await
        .expect("release owner lock barrier");

    let first = first_task
        .await
        .expect("first concurrent install task")
        .expect("first concurrent install");
    let second = second_task
        .await
        .expect("second concurrent install task")
        .expect("exact concurrent replay");
    assert_eq!(first, second);

    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE installation_id = $1",
    )
    .bind(first.installation_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent command ledger row");
    assert_eq!(command_count, 1);
    let durable_rows: (i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(*) FROM ui_installations
              WHERE id = $1 AND project_id = $2 AND repository_id IS NULL),
             (SELECT count(*) FROM ui_installation_generations
              WHERE installation_id = $1)",
    )
    .bind(first.installation_id.as_uuid())
    .bind(fixture.first_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent installation and generation");
    assert_eq!(durable_rows, (1, 1));
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1
           AND event.aggregate_type = 'project'
           AND event.aggregate_id = $2
           AND event.event_type = 'project.changed'",
    )
    .bind(first.idempotency_id)
    .bind(fixture.first_project.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("one concurrent owner event and product outbox");
    assert_eq!(durable_counts, (1, 1));
    println!(
        "REAL_STATIC_INSTALL_REPLAY=1 blocker_pid={owner_backend_pid} command_rows={command_count} \
         installation_rows={} generation_rows={} event_rows={} outbox_rows={}",
        durable_rows.0, durable_rows.1, durable_counts.0, durable_counts.1
    );
}

#[tokio::test]
#[serial]
// This uses the real deferred generation validator and a committed parent
// move; it proves the natural post-insert commit error rolls back every row.
#[allow(clippy::too_many_lines)]
async fn install_static_ui_parent_move_rejects_and_rolls_back_naturally() {
    let Some(admin_pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&admin_pool)
        .await
        .expect("apply application migrations");
    let Some(worker_pool) = worker_pool_named("heph-static-install-rollback").await else {
        return;
    };
    let fixture = seed(&admin_pool).await;
    sqlx::query(
        "UPDATE organization_members
         SET role = 'owner'
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&admin_pool)
    .await
    .expect("promote global installation owner");
    let release_id = publish_static_release(
        &admin_pool,
        &worker_pool,
        &fixture,
        "global",
        "matrix-parent-move-rollback",
    )
    .await;
    let source_project: Uuid = sqlx::query_scalar(
        "SELECT repository.project_id
         FROM releases AS release
         JOIN repositories AS repository ON repository.id = release.repository_id
         WHERE release.id = $1",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("source project");
    let foreign_project = seed_foreign_project(&admin_pool, fixture.actor).await;
    let foreign_organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(foreign_project.as_uuid())
            .fetch_one(&admin_pool)
            .await
            .expect("foreign organization");

    let caller_key =
        UiInstallationCallerKey::parse("matrix-parent-move-rollback").expect("caller key");
    let command_identity = UiInstallationCommandIdentity::new(
        fixture.actor.as_uuid(),
        UiInstallationOperation::Install,
        caller_key.clone(),
    );
    let command_key = command_identity.command_key();
    let occurrence_id =
        actor_idempotency_id(fixture.actor.as_uuid().as_bytes(), command_key.as_bytes()).as_uuid();
    let mut move_tx = admin_pool.begin().await.expect("begin source parent move");
    let move_backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *move_tx)
        .await
        .expect("read source move backend pid");
    sqlx::query("UPDATE projects SET organization_id = $1 WHERE id = $2")
        .bind(foreign_organization)
        .bind(source_project)
        .execute(&mut *move_tx)
        .await
        .expect("hold source project organization move");

    let install_pool = worker_pool.clone();
    let install_actor = fixture.actor;
    let install_target = UiInstallationTarget::organization(fixture.organization);
    let install_task = tokio::spawn(async move {
        ReleaseService::new(install_pool, Arc::new(PostgresMelangeAuthorizer))
            .install_static_ui(
                &identity(install_actor),
                InstallStaticUi {
                    caller_key,
                    target: install_target,
                    release_id,
                    ui_key: release_domain::ui::UiKey::parse("docs").expect("UI key"),
                },
            )
            .await
    });
    wait_for_row_lock_waiters(
        &admin_pool,
        "projects",
        move_backend_pid,
        "heph-static-install-rollback",
        1,
        false,
    )
    .await;
    move_tx
        .commit()
        .await
        .expect("commit source project organization move");
    let result = install_task.await.expect("parent move install task");
    assert!(matches!(
        result,
        Err(UiInstallationError::InvalidOrUnsupported)
    ));

    // Restore the unreferenced fixture parent before checking the durable
    // absence, so a failed assertion cannot leave a cross-tenant fixture.
    sqlx::query("UPDATE projects SET organization_id = $1 WHERE id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(source_project)
        .execute(&admin_pool)
        .await
        .expect("restore source project organization");
    let installation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installations
         WHERE organization_id = $1 AND scope = 'global' AND ui_key = 'docs'",
    )
    .bind(fixture.organization.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back installation absence");
    assert_eq!(installation_count, 0);
    let generation_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_generations
         WHERE release_id = $1 AND ui_key = 'docs'",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back generation absence");
    assert_eq!(generation_count, 0);
    let command_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands
         WHERE actor_id = $1 AND caller_idempotency_key = $2",
    )
    .bind(fixture.actor.as_uuid())
    .bind("matrix-parent-move-rollback")
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back command absence");
    assert_eq!(command_count, 0);
    let durable_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(outbox.event_id)
         FROM application_events AS event
         LEFT JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.occurrence_id = $1",
    )
    .bind(occurrence_id)
    .fetch_one(&admin_pool)
    .await
    .expect("rolled-back event and outbox absence");
    assert_eq!(durable_counts, (0, 0));
    println!(
        "REAL_STATIC_INSTALL_ROLLBACK=1 blocker_pid={move_backend_pid} \
         installation_rows={installation_count} generation_rows={generation_count} \
         command_rows={command_count} event_rows={} outbox_rows={}",
        durable_counts.0, durable_counts.1
    );
}

#[tokio::test]
#[serial]
async fn runtime_git_mode_is_frozen_into_release_and_instance_revision() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let config = reusable_config()
        .replace("mount = true", "mount = false")
        .replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
             [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
             resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
             optional_operations = [\"update_ref\"]\nrequired = true\n\n\
             [capability_slots.git]\nref_globs = [\"refs/heads/content\"]\n\
             changed_path_globs = [\"content/**\"]\nexact_parent_required = false\n\
             transfer = { request_bytes = 1048576, pack_bytes = 8388608, object_count = 10000, ref_updates = 8 }\n\n\
             [state_volume]",
        );
    let git_authority = parse(config.as_bytes())
        .config
        .expect("valid runtime Git config")
        .capability_slots
        .into_iter()
        .find(|slot| slot.key == "content")
        .expect("content slot")
        .git_ceiling()
        .expect("valid Git ceiling")
        .expect("typed Git ceiling");
    let fixture = seed_with_config(&pool, &config).await;
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    service
        .complete_build(CompleteBuild {
            command_key: key("complete-runtime-git", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("runtime-git-v1").expect("release version"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("bin/reviewer").expect("artifact path"),
                kind: ArtifactKind::Executable,
                mode: 0o555,
                content_hash: ContentHash::digest(b"runtime-git-reviewer"),
                size_bytes: 20,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("complete runtime Git release");
    let actor = identity(fixture.actor);
    service
        .publish(
            &actor,
            key("publish-runtime-git", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish runtime Git release");
    let instance_id = AgentInstanceId::new();
    let revision_id = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-runtime-git", instance_id.as_uuid()),
                instance_id,
                revision_id,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("runtime-git-reviewer").expect("instance name"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter name"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("import runtime Git instance");
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)",
    )
    .bind(fixture.first_project.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&pool)
    .await
    .expect("grant capability delegation role");
    let bound_revision = AgentInstanceRevisionId::new();
    let publication_binding = CapabilityBindingId::new();
    service
        .revise_instance_capabilities(
            &actor,
            ReviseInstanceCapabilities {
                command_key: key("bind-runtime-git", bound_revision.as_uuid()),
                instance_id,
                expected_revision_id: revision_id,
                new_revision_id: bound_revision,
                bindings: vec![release_postgres::CapabilityBindingSelection {
                    binding_id: publication_binding,
                    slot: CapabilitySlotKey::parse("content").expect("capability slot"),
                    resource: CapabilityResource::new(
                        CapabilityResourceKind::Repository,
                        fixture.first_aux_repository.as_uuid(),
                    ),
                    granted_operations: vec![
                        CapabilityOperation::GitRead,
                        CapabilityOperation::UpdateRef,
                    ],
                    git_authority: Some(git_authority),
                }],
                authorization_model_version: String::from("melange/v2"),
            },
        )
        .await
        .expect("bind runtime Git repository");
    let trigger_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-distinct-trigger", trigger_attachment.as_uuid()),
                attachment_id: trigger_attachment,
                instance_id,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main").expect("trigger ref"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("create distinct trigger attachment");
    let modes: (String, String, String, Uuid, Uuid) = sqlx::query_as(
        "SELECT release_agent.publication_mode, revision.publication_mode,
                release_agent.publication_repository_slot,
                revision.publication_repository_binding_id,
                binding.resource_id
         FROM release_agents AS release_agent
         JOIN agent_instance_revisions AS revision
           ON revision.release_agent_id = release_agent.id
         JOIN agent_capability_bindings AS binding
           ON binding.id = revision.publication_repository_binding_id
         WHERE release_agent.id = $1 AND revision.id = $2",
    )
    .bind(release_agent_id.as_uuid())
    .bind(bound_revision.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("immutable publication modes");
    assert_eq!(
        modes,
        (
            String::from("runtime_git"),
            String::from("runtime_git"),
            String::from("content"),
            publication_binding.as_uuid(),
            fixture.first_aux_repository.as_uuid(),
        )
    );
    let git_scope: (i16, Vec<String>, Vec<String>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT ceiling.grammar_version, binding.ref_globs,
                binding.changed_path_globs, ceiling.normalized_hash,
                binding.normalized_hash
         FROM release_git_capability_ceilings AS ceiling
         JOIN agent_git_capability_bindings AS binding
           ON binding.requirement_id = ceiling.requirement_id
         WHERE binding.binding_id = $1",
    )
    .bind(publication_binding.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("typed Git ceiling and binding");
    assert_eq!(git_scope.0, 1);
    assert_eq!(git_scope.1, vec![String::from("refs/heads/content")]);
    assert_eq!(git_scope.2, vec![String::from("content/**")]);
    assert_eq!(git_scope.3.len(), 32);
    assert_eq!(git_scope.4.len(), 32);
    assert_ne!(git_scope.3, git_scope.4, "binding hash includes repository");
    let attachment_repository: Uuid =
        sqlx::query_scalar("SELECT repository_id FROM agent_attachments WHERE id = $1")
            .bind(trigger_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("trigger attachment provenance");
    assert_ne!(attachment_repository, modes.4);
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
// The real JetStream pull streams are retained across the gate transition so
// this single integration test can prove transport redelivery and claim CAS.
#[allow(clippy::large_stack_frames)]
async fn publishes_once_and_imports_isolated_instances_with_exact_attachments() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = seed(&pool).await;
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let artifact = ReleaseArtifactInput {
        id: ReleaseArtifactId::new(),
        path: ArtifactPath::parse("bin/reviewer").expect("artifact path should validate"),
        kind: ArtifactKind::Executable,
        mode: 0o555,
        content_hash: ContentHash::digest(b"built-reviewer-v1"),
        size_bytes: 17,
        media_type: String::from("application/octet-stream"),
        storage_key: Uuid::new_v4(),
    };
    let completed = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![artifact],
        })
        .await
        .expect("complete imported build");
    assert_eq!(completed, release_id);
    let stored_publication_mode: String =
        sqlx::query_scalar("SELECT publication_mode FROM release_agents WHERE id = $1")
            .bind(release_agent_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("stored release publication mode");
    assert_eq!(stored_publication_mode, "proposal");
    let mutable_publication_mode =
        sqlx::query("UPDATE release_agents SET publication_mode = 'runtime_git' WHERE id = $1")
            .bind(release_agent_id.as_uuid())
            .execute(&pool)
            .await;
    assert!(
        mutable_publication_mode.is_err(),
        "release publication mode must be immutable"
    );
    let repeated = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("ignored-on-retry")
                    .expect("artifact path should validate"),
                kind: ArtifactKind::File,
                mode: 0o444,
                content_hash: ContentHash::digest(b"ignored"),
                size_bytes: 7,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("duplicate completion should return durable release");
    assert_eq!(repeated, release_id);

    let actor = identity(fixture.actor);
    service
        .publish(&actor, key("publish", release_id.as_uuid()), release_id)
        .await
        .expect("source maintainer should publish");
    let immutable = sqlx::query("UPDATE releases SET source_commit = $2 WHERE id = $1")
        .bind(release_id.as_uuid())
        .bind("b".repeat(40))
        .execute(&pool)
        .await;
    assert!(immutable.is_err(), "published provenance must be immutable");

    let first_instance = AgentInstanceId::new();
    let first_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-first", first_instance.as_uuid()),
                instance_id: first_instance,
                revision_id: first_revision,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("first project should import");
    let revision_publication_mode: String =
        sqlx::query_scalar("SELECT publication_mode FROM agent_instance_revisions WHERE id = $1")
            .bind(first_revision.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("stored revision publication mode");
    assert_eq!(revision_publication_mode, "proposal");
    let second_instance = AgentInstanceId::new();
    let second_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-second", second_instance.as_uuid()),
                instance_id: second_instance,
                revision_id: second_revision,
                project_id: fixture.second_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 1,
                    memory_mib: 512,
                    network: NetworkAccess::Disabled,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("second project should independently import");

    let instances: Vec<(Uuid, Uuid, Uuid, bool)> = sqlx::query_as(
        "SELECT instance.id, instance.project_id, instance.state_volume_id,
                revision.runnable
         FROM agent_instances AS instance
         JOIN agent_instance_revisions AS revision
           ON revision.id = instance.active_revision_id
         WHERE instance.id = ANY($1)
         ORDER BY instance.id",
    )
    .bind(vec![first_instance.as_uuid(), second_instance.as_uuid()])
    .fetch_all(&pool)
    .await
    .expect("stored isolated instances");
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].1, instances[1].1);
    assert_ne!(instances[0].2, instances[1].2);
    assert!(
        instances.iter().all(|row| !row.3),
        "required symbolic secret slot should be visibly unrunnable"
    );
    let consumer_runtime_override_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'agent_instance_revisions'
           AND column_name = ANY($1)",
    )
    .bind(vec![
        "command",
        "arguments",
        "working_directory",
        "image_reference",
        "mounts",
        "requires_state",
    ])
    .fetch_one(&pool)
    .await
    .expect("inspect consumer revision columns");
    assert_eq!(
        consumer_runtime_override_columns, 0,
        "consumer revisions must reference immutable release-owned runtime fields"
    );
    let revised_first = AgentInstanceRevisionId::new();
    service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("revise-first", revised_first.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: revised_first,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("parameter change should create an immutable revision");
    let history: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, parameters ->> 'severity'
         FROM agent_instance_revisions
         WHERE instance_id = $1 ORDER BY created_at, id",
    )
    .bind(first_instance.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("immutable revision history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].1, "warning");
    assert_eq!(history[1].1, "error");
    let stale_revision = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("stale-revise", Uuid::new_v4()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: AgentInstanceRevisionId::new(),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        stale_revision,
        Err(release_postgres::ReleaseServiceError::StaleInstanceRevision)
    ));
    let broadened_revision_id = AgentInstanceRevisionId::new();
    let broadened = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("broaden-policy", broadened_revision_id.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                new_revision_id: broadened_revision_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 5,
                    memory_mib: 1024,
                    network: NetworkAccess::Egress,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        broadened,
        Err(release_postgres::ReleaseServiceError::Domain(
            release_domain::ReleaseValueError::PolicyBroadening
        ))
    ));
    let broadened_persisted: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM agent_instance_revisions WHERE id = $1
         )",
    )
    .bind(broadened_revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect rejected policy revision");
    assert!(!broadened_persisted);
    let unsupported_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("unsupported-update", unsupported_update_id.as_uuid()),
                update_id: unsupported_update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: release_agent_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("unsupported candidate should remain visible");
    let unsupported: (String, bool, bool) = sqlx::query_as(
        "SELECT update.state,
                EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(update.diagnostics) AS item
                    WHERE item->>'code' = 'stateful_update_hook_missing'
                ),
                instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(unsupported_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("unsupported update diagnostics");
    assert_eq!(unsupported.0, "rejected");
    assert!(unsupported.1);
    assert!(unsupported.2);

    let fork_release_agent = seed_fork_release(
        &pool,
        release_id,
        release_agent_id,
        fixture.second_repository,
    )
    .await;
    let fork_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("fork-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: fork_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        fork_update,
        Err(release_postgres::ReleaseServiceError::AgentFamilyMismatch)
    ));

    let update_release_agent = seed_update_release(&pool, release_id, release_agent_id).await;
    let update_release_id: Uuid =
        sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id = $1")
            .bind(update_release_agent.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    let deferred_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-before-update", deferred_attachment.as_uuid()),
                attachment_id: deferred_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_aux_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("pre-update attachment");
    let deferred_receive = Uuid::new_v4();
    let deferred_commit = "e".repeat(40);
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'deferred-test', 'accepted', now())",
    )
    .bind(deferred_receive)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&pool)
    .await
    .expect("seed exact target receive");
    let prior_request_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          run_id, command_id, instance_id, instance_revision_id,
          release_id, release_agent_id, attachment_id, request_kind,
          platform_policy_version, requires_state, dispatch_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                 $9, $10, $11, 'instance_normal', 'platform/v2', true,
                 'pending')",
    )
    .bind(prior_request_id)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(revised_first.as_uuid())
    .bind(release_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("seed prior exact-revision request");
    let update_id = AgentUpdateId::new();
    let update_candidate_revision = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("valid-update", update_id.as_uuid()),
                update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: update_candidate_revision,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("valid stateful candidate should close the gate");
    // Reproduce the production race through both adapters: the accepted
    // mailbox dispatch is consumed while the update gate is closed, then the
    // release service must commit a fresh wake when activation reopens it.
    let mailbox_store = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    mailbox_store
        .ensure_mailbox(fixture.first_project.as_uuid(), mailbox_id, first_instance)
        .await
        .expect("create update-race mailbox");
    let mailbox_body = b"release-update-gate-race";
    let mailbox_event = gate_race_event(
        mailbox_id,
        first_instance,
        mailbox_body,
        "release-update-race",
    );
    let accepted = mailbox_store
        .accept(
            fixture.first_project.as_uuid(),
            &mailbox_event,
            mailbox_body,
            u32::try_from(mailbox_body.len()).expect("bounded body"),
        )
        .await
        .expect("accept update-race mailbox event");
    let wake = MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationId::from_uuid(accepted.event_id.as_uuid()),
        event_id: accepted.event_id,
    };
    mailbox_store
        .apply_command(MAILBOX_WAKE_SUBJECT, &wake)
        .await
        .expect("apply initial update-race wake");
    let dispatch = MailboxDispatchCommand {
        operation_id: mailbox_domain::MailboxOperationIdentity::dispatch(
            mailbox_id,
            accepted.event_id,
            1,
        )
        .id(),
        event_id: accepted.event_id,
    };
    mailbox_store
        .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch)
        .await
        .expect("verify initial update-race dispatch");
    assert!(
        mailbox_store
            .claim_dispatch(&dispatch)
            .await
            .expect("closed-gate update-race claim")
            .is_none(),
        "the pre-activation dispatch must not create an attempt"
    );
    let nats_event_id = if let Ok(nats_url) = std::env::var("HEPHAESTUS_NATS_TEST_URL") {
        let nats_body = b"release-update-nats-gate-race";
        let nats_event = gate_race_event(
            mailbox_id,
            first_instance,
            nats_body,
            "release-update-nats-race",
        );
        let accepted = mailbox_store
            .accept(
                fixture.first_project.as_uuid(),
                &nats_event,
                nats_body,
                u32::try_from(nats_body.len()).expect("bounded NATS body"),
            )
            .await
            .expect("accept NATS update-race mailbox event");
        let nats = async_nats::connect(nats_url)
            .await
            .expect("connect update-race NATS");
        let jetstream = async_nats::jetstream::new(nats);
        let consumer = mailbox_dispatch::ensure_mailbox_jetstream_topology(&jetstream)
            .await
            .expect("create update-race NATS topology");
        let publisher = mailbox_dispatch::MailboxOutboxPublisher::new(
            jetstream,
            Arc::new(mailbox_store.clone()),
        );
        publisher
            .publish_pending(100)
            .await
            .expect("publish pre-activation update-race commands");
        let mut messages = consumer
            .messages()
            .await
            .expect("open update-race consumer");
        let wake_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive pre-activation NATS wake")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS wake command");
            if delivery.message.subject.as_str() == MAILBOX_WAKE_SUBJECT
                && command.event_id == accepted.event_id
            {
                break delivery;
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(
                MAILBOX_WAKE_SUBJECT,
                &serde_json::from_slice(&wake_delivery.payload).expect("wake command"),
            )
            .await
            .expect("apply pre-activation NATS wake");
        wake_delivery
            .double_ack()
            .await
            .expect("ack pre-activation NATS wake");
        publisher
            .publish_pending(100)
            .await
            .expect("publish pre-activation NATS dispatch");
        let dispatch_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive pre-activation NATS dispatch")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS dispatch command");
            if delivery.message.subject.as_str() == MAILBOX_DISPATCH_SUBJECT
                && command.event_id == accepted.event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_delivery.1)
            .await
            .expect("apply pre-activation NATS dispatch");
        assert!(
            mailbox_store
                .claim_dispatch(&dispatch_delivery.1)
                .await
                .expect("pre-activation NATS claim")
                .is_none(),
            "pre-activation NATS dispatch must not create an attempt"
        );
        dispatch_delivery
            .0
            .double_ack()
            .await
            .expect("ack consumed pre-activation NATS dispatch");
        drop(messages);
        Some(accepted.event_id)
    } else {
        None
    };
    let concurrent_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("concurrent-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        concurrent_update,
        Err(release_postgres::ReleaseServiceError::ConcurrentUpdate)
    ));
    let deferred_trigger_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deferred_agent_triggers
         (id, instance_id, attachment_id, repository_id, target_ref,
          target_commit, source_id)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6)",
    )
    .bind(deferred_trigger_id)
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("defer trigger behind closed gate");
    let draining: (String, String, bool) = sqlx::query_as(
        "SELECT update.state, instance.state, instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("draining update");
    assert_eq!(
        draining,
        (
            String::from("draining"),
            String::from("update_draining"),
            false
        )
    );
    let drain_probe = service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("drain-probe", update_id.as_uuid()),
                update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await;
    assert!(matches!(
        drain_probe,
        Err(release_postgres::ReleaseServiceError::UpdateDrainPending)
    ));
    sqlx::query(
        "UPDATE run_requests SET dispatch_state = 'dispatched'
         WHERE id = $1",
    )
    .bind(prior_request_id)
    .execute(&pool)
    .await
    .expect("drain pre-gate request");
    let volume_id: Uuid =
        sqlx::query_scalar("SELECT state_volume_id FROM agent_instances WHERE id = $1")
            .bind(first_instance.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("instance state volume");
    sqlx::query(
        "UPDATE agent_instance_state_volumes
         SET state = 'ready', host_id = 'test-host',
             host_path = $2, filesystem_uuid = $3
         WHERE id = $1",
    )
    .bind(volume_id)
    .bind(format!("/var/lib/hephaestus-test/{volume_id}"))
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("allocate update volume fixture");
    let hook_run_id = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("drained update should acquire the fenced volume");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("duplicate hook admission should resolve idempotently");
    let update_start_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("one exact update start command");
    assert_eq!(update_start_count, 1);
    let update_start: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact update start command");
    assert_eq!(update_start["kind"], "update");
    assert_eq!(update_start["attachment_id"], serde_json::Value::Null);
    assert_eq!(update_start["run_id"], hook_run_id.to_string());
    assert_eq!(update_start["requires_state"], true);
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', exit_code = 0,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(hook_run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("persist cleaned successful update run");
    let activated = service
        .reconcile_update_run(hook_run_id)
        .await
        .expect("reconcile and activate exact committed candidate");
    assert_eq!(activated, UpdateDecision::Activated);
    let activation_wakes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation mailbox wake");
    assert_eq!(
        activation_wakes, 1,
        "activation must re-wake eligible delivery"
    );
    let activation_wake_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
         ORDER BY occurred_at DESC, id DESC LIMIT 1",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load activation mailbox wake");
    mailbox_store
        .apply_command(
            MAILBOX_WAKE_SUBJECT,
            &MailboxDispatchCommand {
                operation_id: mailbox_domain::MailboxOperationId::from_uuid(activation_wake_id),
                event_id: accepted.event_id,
            },
        )
        .await
        .expect("apply activation mailbox wake");
    let activation_dispatches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2",
    )
    .bind(MAILBOX_DISPATCH_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation dispatch commands");
    assert_eq!(
        activation_dispatches, 2,
        "activation must enqueue one fresh dispatch"
    );
    sqlx::query(
        "INSERT INTO git_refs
         (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, 'refs/heads/main', $2, $3)",
    )
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("seed exact update-race target ref");
    let attempts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count update-race attempts");
    assert_eq!(
        attempts, 0,
        "wake recovery must not create an attempt itself"
    );
    let resumed = mailbox_store
        .claim_dispatch(&dispatch)
        .await
        .expect("claim post-activation update-race dispatch")
        .expect("fresh transport identity claims the candidate once");
    assert_eq!(
        resumed.instance_revision_id, update_candidate_revision,
        "post-activation dispatch must use the active candidate revision"
    );
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', updated_at = now()
         WHERE id = $1",
    )
    .bind(resumed.run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("finish update-race proof run");
    assert!(
        mailbox_store
            .claim_dispatch(&dispatch)
            .await
            .expect("reject duplicate update-race dispatch")
            .is_none(),
        "a stale or duplicate transport command must not create another attempt"
    );
    if let Some(nats_event_id) = nats_event_id {
        let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL").expect("NATS URL remains set");
        let nats = async_nats::connect(nats_url)
            .await
            .expect("reconnect update-race NATS");
        let jetstream = async_nats::jetstream::new(nats);
        let consumer = mailbox_dispatch::ensure_mailbox_jetstream_topology(&jetstream)
            .await
            .expect("reopen update-race NATS topology");
        let publisher = mailbox_dispatch::MailboxOutboxPublisher::new(
            jetstream,
            Arc::new(mailbox_store.clone()),
        );
        let activation_wake_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_WAKE_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation wake");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS wake");
        let wake_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_wake_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS wake");
        assert!(wake_published, "activation wake must be accepted by NATS");
        let mut messages = consumer.messages().await.expect("open NATS consumer");
        let wake_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS wake")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS activation wake command");
            if delivery.message.subject.as_str() == MAILBOX_WAKE_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(MAILBOX_WAKE_SUBJECT, &wake_delivery.1)
            .await
            .expect("apply post-activation NATS wake");
        wake_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS wake");
        let expected_operation_id =
            MailboxOperationIdentity::dispatch(mailbox_id, nats_event_id, 1).id();
        let activation_dispatch_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $3
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_DISPATCH_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .bind(expected_operation_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation dispatch");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS dispatch");
        let dispatch_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_dispatch_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS dispatch");
        assert!(
            dispatch_published,
            "activation dispatch must be accepted by NATS"
        );
        let dispatch_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS dispatch")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand = serde_json::from_slice(&delivery.payload)
                .expect("NATS activation dispatch command");
            if delivery.message.subject.as_str() == MAILBOX_DISPATCH_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        assert_eq!(dispatch_delivery.1.operation_id, expected_operation_id);
        mailbox_store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_delivery.1)
            .await
            .expect("apply post-activation NATS dispatch");
        let nats_run = mailbox_store
            .claim_dispatch(&dispatch_delivery.1)
            .await
            .expect("claim post-activation NATS dispatch")
            .expect("NATS re-wake claims candidate");
        assert_eq!(nats_run.instance_revision_id, update_candidate_revision);
        assert!(
            mailbox_store
                .claim_dispatch(&dispatch_delivery.1)
                .await
                .expect("duplicate post-activation NATS claim")
                .is_none()
        );
        sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
            .bind(nats_run.run_id.as_uuid())
            .execute(&pool)
            .await
            .expect("finish post-activation NATS proof run");
        dispatch_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS dispatch");
    }
    let active_after_update: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active candidate");
    assert_eq!(
        active_after_update,
        (
            update_candidate_revision.as_uuid(),
            String::from("active"),
            true
        )
    );
    let completed_update: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'active'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical completed update event");
    assert!(completed_update);
    let identity_preserved: (Uuid, bool) = sqlx::query_as(
        "SELECT instance.state_volume_id,
                EXISTS(
                    SELECT 1 FROM agent_attachments
                    WHERE id = $2 AND instance_id = instance.id
                )
         FROM agent_instances AS instance WHERE instance.id = $1",
    )
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("instance identity after update");
    assert_eq!(identity_preserved, (volume_id, true));
    let volume_ready: String =
        sqlx::query_scalar("SELECT state FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id)
            .fetch_one(&pool)
            .await
            .expect("released update volume");
    assert_eq!(volume_ready, "ready");
    let materialized_deferred: (String, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT deferred.state, request.instance_revision_id,
                request.release_agent_id, request.commit_sha
         FROM deferred_agent_triggers AS deferred
         JOIN run_requests AS request ON request.id = deferred.run_request_id
         WHERE deferred.id = $1",
    )
    .bind(deferred_trigger_id)
    .fetch_one(&pool)
    .await
    .expect("materialized deferred trigger");
    assert_eq!(
        materialized_deferred,
        (
            String::from("materialized"),
            update_candidate_revision.as_uuid(),
            update_release_agent.as_uuid(),
            deferred_commit,
        ),
        "deferred work must bind only the revision active after gate reopen"
    );
    let exact_revision_requests: Vec<(Uuid, Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT request.instance_revision_id, request.release_id,
                revision.parameter_hash
         FROM run_requests AS request
         JOIN agent_instance_revisions AS revision
           ON revision.id = request.instance_revision_id
         WHERE request.receive_id = $1",
    )
    .bind(deferred_receive)
    .fetch_all(&pool)
    .await
    .expect("exact requests across active revisions");
    assert_eq!(exact_revision_requests.len(), 2);
    let prior_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == revised_first.as_uuid())
        .expect("prior revision request");
    let candidate_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == update_candidate_revision.as_uuid())
        .expect("candidate revision request");
    assert_eq!(prior_request.1, release_id.as_uuid());
    assert_ne!(prior_request.1, candidate_request.1);
    assert_ne!(prior_request.2, candidate_request.2);
    sqlx::query(
        "UPDATE run_requests
         SET dispatch_state = 'dispatched'
         WHERE id = (
             SELECT run_request_id
             FROM deferred_agent_triggers WHERE id = $1
         )",
    )
    .bind(deferred_trigger_id)
    .execute(&pool)
    .await
    .expect("simulate deferred request dispatch");

    let rejected_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("agent-rejected-update", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("agent-rejected candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("agent-rejected-hook", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("agent-rejected hook");
    assert_eq!(
        service
            .record_update_hook_result(rejected_update_id, UpdateHookResult::Rejected(23))
            .await
            .expect("explicit agent rollback result"),
        UpdateDecision::AgentRejected
    );
    let agent_rejected: (Uuid, String, bool, i32) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.hook_exit_code
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(rejected_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("agent rejection state");
    assert_eq!(
        agent_rejected,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            23,
        )
    );
    let rejected_event: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'rejected'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical update rejection event");
    assert!(rejected_event);

    let retry_update_id = AgentUpdateId::new();
    let retry_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("retry-update", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: retry_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("second update candidate");
    let retry_first_run = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-first-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: retry_first_run,
            },
        )
        .await
        .expect("first uncertain attempt");
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'failed', exit_signal = 9,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(retry_first_run.as_uuid())
    .execute(&pool)
    .await
    .expect("persist signal-terminated update run");
    assert_eq!(
        service
            .reconcile_update_run(retry_first_run)
            .await
            .expect("signal failure pauses uncertain update"),
        UpdateDecision::CompatibilityUnknown
    );
    let uncertain_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'
           AND safe_state = 'paused'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical uncertain update events");
    assert!(uncertain_events >= 2);
    let retry_command_key = key("retry-recovery", retry_update_id.as_uuid());
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("operator-authorized retry"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("retry recovery command is idempotent"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-second-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("retry uses the same update identity");
    service
        .record_update_hook_result(retry_update_id, UpdateHookResult::Uncertain)
        .await
        .expect("second uncertain attempt");
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("reject-recovery", retry_update_id.as_uuid()),
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RejectCandidate,
                },
            )
            .await
            .expect("operator rejects uncertain candidate"),
        UpdateRecoveryDecision::CandidateRejected
    );
    let rejected_recovery: (Uuid, String, bool, String) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.final_decision
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(retry_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("rejected recovery state");
    assert_eq!(
        rejected_recovery,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            String::from("recovery"),
        )
    );

    let resume_update_id = AgentUpdateId::new();
    let resume_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("resume-update", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: resume_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("activation-recovery candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("resume-hook", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("activation-recovery hook");
    service
        .record_update_hook_result(resume_update_id, UpdateHookResult::Committed)
        .await
        .expect("durable hook commit");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("revocation after the hook commit point");
    sqlx::query("UPDATE agent_instances SET state = 'recovering' WHERE id = $1")
        .bind(first_instance.as_uuid())
        .execute(&pool)
        .await
        .expect("simulate activation CAS anomaly");
    assert_eq!(
        service
            .activate_committed_update(resume_update_id)
            .await
            .expect("activation anomaly becomes recovery"),
        UpdateDecision::ActivationRecovery
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("resume-recovery", resume_update_id.as_uuid()),
                    update_id: resume_update_id,
                    action: UpdateRecoveryAction::ResumeActivation,
                },
            )
            .await
            .expect("operator resumes durable activation"),
        UpdateRecoveryDecision::CandidateActivated
    );
    let resumed: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("resumed candidate");
    assert_eq!(
        resumed,
        (resume_candidate.as_uuid(), String::from("active"), true)
    );

    let first_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("same-project attachment should succeed");
    let historical_run = RunId::new();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, run_kind, state, outcome,
          exit_code, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'normal', 'cleaned_up',
                 'succeeded', 0, true, now(), now())",
    )
    .bind(historical_run.as_uuid())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(resume_candidate.as_uuid())
    .bind(update_release_id)
    .bind(update_release_agent.as_uuid())
    .bind(first_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("historical normal run");
    let second_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-second", second_attachment.as_uuid()),
                attachment_id: second_attachment,
                instance_id: second_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/release/*")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::PushAndManual,
            },
        )
        .await
        .expect("second same-project attachment should succeed");
    let attachment_isolation: (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT attachment.id)::bigint,
                count(DISTINCT instance.state_volume_id)::bigint
         FROM agent_attachments AS attachment
         JOIN agent_instances AS instance ON instance.id = attachment.instance_id
         WHERE attachment.instance_id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("shared instance state across attachments");
    assert_eq!(
        attachment_isolation,
        (2, 1),
        "two attachments of one instance must share its one state volume"
    );
    let cross_project_attachment = AgentAttachmentId::new();
    let cross_project = service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-cross", Uuid::new_v4()),
                attachment_id: cross_project_attachment,
                instance_id: first_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Manual,
            },
        )
        .await;
    assert!(cross_project.is_err());
    let rolled_back_messages: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE aggregate_id = $1")
            .bind(cross_project_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rolled-back command has no message");
    assert_eq!(rolled_back_messages, 0);
    service
        .set_attachment_enabled(
            &actor,
            SetAttachmentEnabled {
                command_key: key("disable-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                enabled: false,
            },
        )
        .await
        .expect("authorized attachment disable");
    let disabled: bool =
        sqlx::query_scalar("SELECT NOT enabled FROM agent_attachments WHERE id = $1")
            .bind(first_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("disabled attachment");
    assert!(disabled);
    service
        .remove_attachment(
            &actor,
            RemoveAttachment {
                command_key: key("remove-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
            },
        )
        .await
        .expect("authorized attachment tombstone");
    let tombstoned: bool = sqlx::query_scalar(
        "SELECT removed_at IS NOT NULL AND NOT enabled
         FROM agent_attachments WHERE id = $1",
    )
    .bind(first_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("tombstoned attachment");
    assert!(tombstoned);
    let attachment_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical attachment invalidations");
    assert!(attachment_events >= 3);
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("authorized release revocation");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("release revocation replay is idempotent");
    let preserved_history: (Uuid, Uuid, Uuid, Uuid, bool, String) = sqlx::query_as(
        "SELECT run.id, revision.id, release_agent.id, release.id,
                attachment.removed_at IS NOT NULL, release.state
         FROM runs AS run
         JOIN agent_instance_revisions AS revision
           ON revision.id = run.instance_revision_id
          AND revision.instance_id = run.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = run.release_agent_id
          AND release_agent.release_id = run.release_id
         JOIN releases AS release ON release.id = run.release_id
         JOIN agent_attachments AS attachment
           ON attachment.id = run.attachment_id
          AND attachment.instance_id = run.instance_id
         WHERE run.id = $1",
    )
    .bind(historical_run.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("historical foreign-key targets after tombstones");
    assert_eq!(
        preserved_history,
        (
            historical_run.as_uuid(),
            resume_candidate.as_uuid(),
            update_release_agent.as_uuid(),
            update_release_id,
            true,
            String::from("revoked"),
        )
    );
    let revocation_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'release' AND aggregate_id = $1
           AND event_type = 'release.changed' AND safe_state = 'revoked'",
    )
    .bind(update_release_id)
    .fetch_one(&pool)
    .await
    .expect("one release revocation event");
    assert_eq!(revocation_events, 1);

    let artifact_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_artifacts WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("artifact count");
    assert_eq!(artifact_count, 1);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_id IN ($1, $2, $3)",
    )
    .bind(release_id.as_uuid())
    .bind(first_instance.as_uuid())
    .bind(second_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable event count");
    assert!(event_count >= 4);
}

#[tokio::test]
#[serial]
async fn release_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some(pool)) = (std::env::var("HEPHAESTUS_NATS_TEST_URL"), pool().await)
    else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'release' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate release outbox fixture");
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES
         ($1, 'release', $2, 'hephaestus.instance.run.requested.v1',
          'instance.run.requested.v1', '{}', now()),
         ($3, 'release', $4, 'hephaestus.run.start',
          'run.start.v1', '{}', now())",
    )
    .bind(first_id)
    .bind(Uuid::new_v4())
    .bind(second_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert release outbox fixture");

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_RELEASE_TEST_{}", first_id.simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![String::from("hephaestus.>")],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated release stream");
    let publisher = ReleaseOutboxPublisher::new(context.clone(), pool.clone());
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("first publication"),
        2
    );
    assert_eq!(stream.info().await.expect("stream state").state.messages, 2);
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("retry publication"),
        2
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        2
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    sqlx::query("DELETE FROM outbox WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("clean release outbox fixture");
}
