use authz_postgres::begin_actor_transaction;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use mailbox_dispatch::MailboxDispatchStore;
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId,
    MailboxOperationIdentity, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn seed_running_run(
    store: &PostgresMailboxRepository,
    pool: &sqlx::PgPool,
    project_id: Uuid,
    mailbox_id: MailboxId,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    body: &[u8],
) -> (MailboxEventId, Uuid) {
    let mut incoming = event(mailbox_id, instance_id, body);
    incoming.deduplication_key = DeduplicationKey::parse(format!("race-{}", Uuid::new_v4()))
        .expect("race-test deduplication key");
    let accepted = store
        .accept(
            project_id,
            &incoming,
            body,
            u32::try_from(body.len()).expect("body length"),
        )
        .await
        .expect("accept race-test event");
    sqlx::query(
        "UPDATE mailbox_deliveries SET disposition = 'eligible', next_eligible_at = NULL
         WHERE event_id = $1",
    )
    .bind(accepted.event_id.as_uuid())
    .execute(pool)
    .await
    .expect("make race-test delivery eligible");
    let command = mailbox_dispatch::MailboxDispatchCommand {
        operation_id: MailboxOperationIdentity::dispatch(mailbox_id, accepted.event_id, 1).id(),
        event_id: accepted.event_id,
    };
    let run = store
        .claim_dispatch(&command)
        .await
        .expect("claim race-test run")
        .expect("race-test run exists");
    attach_run_snapshot(
        pool,
        run.run_id.as_uuid(),
        instance_id,
        revision_id,
        "mailbox-recovery-race/v1",
        7,
    )
    .await;
    sqlx::query(
        "UPDATE mailbox_delivery_attempts SET state = 'running'
         WHERE run_id = $1",
    )
    .bind(run.run_id.as_uuid())
    .execute(pool)
    .await
    .expect("mark race-test attempt running");
    (accepted.event_id, run.run_id.as_uuid())
}

pub async fn attach_run_snapshot(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    model_version: &str,
    hash_byte: u8,
) {
    let snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
             (id, run_id, instance_id, instance_revision_id,
              authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(model_version)
    .bind([hash_byte; 32].as_slice())
    .execute(pool)
    .await
    .expect("persist race-test authorization snapshot");
    sqlx::query(
        "UPDATE mailbox_delivery_attempts
         SET authorization_snapshot_id = $2
         WHERE run_id = $1",
    )
    .bind(run_id)
    .bind(snapshot_id)
    .execute(pool)
    .await
    .expect("record race-test authorization snapshot");
}

pub async fn visible_event_count(
    pool: &sqlx::PgPool,
    identity: &AuthenticatedIdentity,
    mailbox_id: MailboxId,
) -> i64 {
    let mut transaction = begin_actor_transaction(pool, identity)
        .await
        .expect("set authenticated actor context");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *transaction)
        .await
        .expect("use non-bypass application role");
    let count = sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
        .bind(mailbox_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .expect("RLS mailbox event inspection");
    transaction.commit().await.expect("commit inspection");
    count
}

pub fn event(mailbox_id: MailboxId, instance_id: AgentInstanceId, body: &[u8]) -> MailboxEvent {
    MailboxEvent {
        id: MailboxEventId::new(),
        mailbox_id,
        instance_id,
        producer_id: ProducerId::parse("real-postgres-nats").expect("producer"),
        deduplication_key: DeduplicationKey::parse("delivery-1").expect("dedup key"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/proof").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(body.len()).expect("body length"),
                    Sha256::digest(body).into(),
                )
                .expect("body reference"),
                Some("application/octet-stream".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
    }
}

pub struct Fixture {
    pub project: Uuid,
    pub instance: AgentInstanceId,
    pub revision: AgentInstanceRevisionId,
    pub release_agent: ReleaseAgentId,
    pub owner: AuthenticatedIdentity,
    pub outsider: AuthenticatedIdentity,
}

// Keep this relational fixture together so its foreign-key setup remains easy
// to audit as one coherent PostgreSQL scenario.
#[allow(clippy::too_many_lines)]
pub async fn seed_instance(pool: &sqlx::PgPool) -> Fixture {
    let owner = create_user(pool, "Mailbox owner").await;
    let outsider = create_user(pool, "Mailbox outsider").await;
    let organization_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let build_request_id = Uuid::new_v4();
    let instance_id = AgentInstanceId::new();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("mailbox-{organization_id}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(owner.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("mailbox-{project_id}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("mailbox-{repository_id}"))
        .execute(pool)
        .await
        .expect("repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, 'mailbox')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("family");
    sqlx::query("INSERT INTO build_requests (id, repository_id, source_commit, source_ref, build_definition_hash, state, completed_at) VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())")
        .bind(build_request_id).bind(repository_id).bind("a".repeat(40)).bind([1_u8; 32].as_slice()).execute(pool).await.expect("build");
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'published', now())")
        .bind(release_id.as_uuid()).bind(repository_id).bind(format!("mailbox-{release_id}")).bind("a".repeat(40)).bind(build_request_id).bind([1_u8; 32].as_slice()).bind([2_u8; 32].as_slice()).bind([3_u8; 32].as_slice()).execute(pool).await.expect("release");
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, requires_state) VALUES ($1, $2, $3, 'mailbox', 'Mailbox', '{}', $4, false)")
        .bind(release_agent_id.as_uuid()).bind(release_id.as_uuid()).bind(family_id).bind([4_u8; 32].as_slice()).execute(pool).await.expect("release agent");
    sqlx::query("INSERT INTO agent_instances (id, project_id, family_id, name, state) VALUES ($1, $2, $3, $4, 'active')")
        .bind(instance_id.as_uuid()).bind(project_id).bind(family_id).bind(format!("mailbox-{instance_id}")).execute(pool).await.expect("instance");
    sqlx::query("INSERT INTO agent_instance_revisions (id, instance_id, release_agent_id, parameters, parameter_hash, resource_selection, network_restriction, effective_runtime_policy, effective_policy_hash, platform_policy_version, runnable) VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)")
        .bind(revision_id.as_uuid()).bind(instance_id.as_uuid()).bind(release_agent_id.as_uuid()).bind([5_u8; 32].as_slice()).bind([6_u8; 32].as_slice()).execute(pool).await.expect("revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query("INSERT INTO agent_attachments (id, instance_id, project_id, repository_id, ref_selector, trigger_policy) VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')")
        .bind(AgentAttachmentId::new().as_uuid()).bind(instance_id.as_uuid()).bind(project_id).bind(repository_id).execute(pool).await.expect("attachment");
    let receive_id = Uuid::new_v4();
    sqlx::query("INSERT INTO git_receives (id, repository_id, principal, status, accepted_at) VALUES ($1, $2, 'mailbox-test', 'accepted', now())")
        .bind(receive_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("receive");
    sqlx::query("INSERT INTO git_refs (repository_id, git_ref, commit_sha, updated_by_receive_id) VALUES ($1, 'refs/heads/main', $2, $3)")
        .bind(repository_id)
        .bind("a".repeat(40))
        .bind(receive_id)
        .execute(pool)
        .await
        .expect("target ref");
    Fixture {
        project: project_id,
        instance: instance_id,
        revision: revision_id,
        release_agent: release_agent_id,
        owner,
        outsider,
    }
}
pub async fn create_user(pool: &sqlx::PgPool, display_name: &str) -> AuthenticatedIdentity {
    let user_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.as_uuid())
        .bind(display_name)
        .execute(pool)
        .await
        .expect("create mailbox test user");
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("mailbox-test-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
