use run_domain::{RunKind, StartRun};
use runtime_types::{CommandId, RunId};
use sha2::{Digest, Sha256};

// Keeping the complete exact-provenance graph together makes this fixture auditable.
#[allow(clippy::too_many_lines)]
pub async fn seed_instance(pool: &sqlx::PgPool, command: &StartRun) {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_request_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("runtime-{organization_id}"))
        .execute(pool)
        .await
        .expect("runtime organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("runtime-{project_id}"))
        .execute(pool)
        .await
        .expect("runtime project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, $3)",
    )
    .bind(repository_id)
    .bind(project_id)
    .bind(format!("runtime-{repository_id}"))
    .execute(pool)
    .await
    .expect("runtime repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'runtime')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("runtime family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_request_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', now())",
    )
    .bind(command.release_id.as_uuid())
    .bind(repository_id)
    .bind(format!("test-{}", command.release_id))
    .bind("a".repeat(40))
    .bind(build_request_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'runtime', 'Runtime', '{}', $4, $5)",
    )
    .bind(command.release_agent_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .bind(command.requires_state)
    .execute(pool)
    .await
    .expect("runtime release agent");
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(command.instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("runtime-{}", command.instance_id))
    .execute(pool)
    .await
    .expect("runtime instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)",
    )
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime revision");
    sqlx::query(
        "UPDATE agent_instances SET active_revision_id = $2
         WHERE id = $1",
    )
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .execute(pool)
    .await
    .expect("activate runtime revision");
    if let Some(attachment_id) = command.attachment_id {
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
        )
        .bind(attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("runtime attachment");
    }
}

pub async fn seed_mailbox_input(pool: &sqlx::PgPool, command: &StartRun, body: &[u8]) {
    let (project_id, _) = seed_run_request_scope(pool, command).await;
    let mailbox_id = uuid::Uuid::new_v4();
    let payload_id = uuid::Uuid::new_v4();
    let event_id = uuid::Uuid::new_v4();
    let integrity_hash: [u8; 32] = Sha256::digest(body).into();
    sqlx::query(
        "INSERT INTO mailboxes (id, project_id, instance_id, state)
         VALUES ($1, $2, $3, 'active')",
    )
    .bind(mailbox_id)
    .bind(project_id)
    .bind(command.instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("mailbox input mailbox");
    sqlx::query(
        "INSERT INTO mailbox_payloads
         (id, mailbox_id, project_id, encoded_body, encoded_length,
          decoded_length, integrity_hash)
         VALUES ($1, $2, $3, $4, $5, $5, $6)",
    )
    .bind(payload_id)
    .bind(mailbox_id)
    .bind(project_id)
    .bind(body)
    .bind(i32::try_from(body.len()).expect("bounded test body"))
    .bind(integrity_hash.as_slice())
    .execute(pool)
    .await
    .expect("mailbox input payload");
    sqlx::query(
        "INSERT INTO mailbox_events
         (id, mailbox_id, project_id, instance_id, body_id, producer_kind,
          producer_id, deduplication_scope, deduplication_key, method, route,
          received_at)
         VALUES ($1, $2, $3, $4, $5, 'gateway', 'retry-test', 'retry-test',
                 $6, 'POST', '/retry-test', now())",
    )
    .bind(event_id)
    .bind(mailbox_id)
    .bind(project_id)
    .bind(command.instance_id.as_uuid())
    .bind(payload_id)
    .bind(format!("event-{event_id}"))
    .execute(pool)
    .await
    .expect("mailbox input event");
    sqlx::query(
        "INSERT INTO mailbox_delivery_attempts
         (id, event_id, mailbox_id, attempt_number, state, command_id,
          instance_id, instance_revision_id, run_id, target_ref, target_commit,
          state_access_outcome)
         VALUES ($1, $2, $3, 1, 'leased', $4, $5, $6, $7,
                 'refs/heads/main', $8, 'no_state')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(event_id)
    .bind(mailbox_id)
    .bind(uuid::Uuid::new_v4())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.run_id.as_uuid())
    .bind("a".repeat(40))
    .execute(pool)
    .await
    .expect("mailbox input attempt");
}

pub async fn seed_run_request_scope(
    pool: &sqlx::PgPool,
    command: &StartRun,
) -> (uuid::Uuid, uuid::Uuid) {
    sqlx::query_as(
        "SELECT instance.project_id, attachment.repository_id
           FROM agent_instances instance
           JOIN agent_attachments attachment ON attachment.instance_id = instance.id
          WHERE instance.id = $1 AND attachment.id = $2",
    )
    .bind(command.instance_id.as_uuid())
    .bind(command.attachment_id.expect("run attachment").as_uuid())
    .fetch_one(pool)
    .await
    .expect("run request scope")
}

pub fn retry_command(source: &StartRun) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: source.instance_id,
        instance_revision_id: source.instance_revision_id,
        release_id: source.release_id,
        release_agent_id: source.release_agent_id,
        attachment_id: source.attachment_id,
        kind: RunKind::Normal,
        requires_state: source.requires_state,
    }
}

pub async fn seed_retry_request(
    pool: &sqlx::PgPool,
    command: &StartRun,
    repository_id: uuid::Uuid,
    source_run_id: RunId,
) {
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, run_id, command_id,
          trigger_command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, platform_policy_version,
          request_kind, requires_state, retry_of_run_id, attempt)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $5, $6, $7, $8,
                 $9, $10, 'test/v1', 'instance_normal', $11, $12, $13)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(command.run_id.as_uuid())
    .bind(command.command_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind(command.attachment_id.expect("retry attachment").as_uuid())
    .bind(command.requires_state)
    .bind(source_run_id.as_uuid())
    .bind(2_i32)
    .execute(pool)
    .await
    .expect("seed retry run request");
}

pub async fn seed_run_request(pool: &sqlx::PgPool, command: &StartRun) -> (uuid::Uuid, uuid::Uuid) {
    let (project_id, repository_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT instance.project_id, attachment.repository_id
         FROM agent_instances instance
         JOIN agent_attachments attachment ON attachment.instance_id = instance.id
         WHERE instance.id = $1 AND attachment.id = $2",
    )
    .bind(command.instance_id.as_uuid())
    .bind(
        command
            .attachment_id
            .expect("normal run attachment")
            .as_uuid(),
    )
    .fetch_one(pool)
    .await
    .expect("load run request scope fixture");
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, run_id, command_id,
          trigger_command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, platform_policy_version,
          request_kind, requires_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $5, $6, $7, $8,
                 $9, $10, 'test/v1', 'instance_normal', $11)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(command.run_id.as_uuid())
    .bind(command.command_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind(
        command
            .attachment_id
            .expect("normal run attachment")
            .as_uuid(),
    )
    .bind(command.requires_state)
    .execute(pool)
    .await
    .expect("seed run request before run");
    (project_id, repository_id)
}
