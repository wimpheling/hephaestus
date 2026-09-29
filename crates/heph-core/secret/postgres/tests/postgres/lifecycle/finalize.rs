use crate::support::{FakeBroker, LifecycleState, SENTINEL, key, seed_queued_run};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{CommitSha, GitRef};
use release_domain::AgentAttachmentId;
use secret_application::{BrokerRequest, ResolveRunSecrets, SecretServiceError};
use secret_domain::{ExecutionPhase, SecretRuntimeSessionId, SecretSlotKey};
use secret_postgres::SecretRuntimeService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::sync::{Arc, atomic::AtomicBool};
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn finalize_and_purge(state: &LifecycleState) {
    let pool = state.pool.clone();
    let service = Arc::clone(&state.service);
    let owner = state.owner.clone();
    let target_manager = state.target_manager.clone();
    let secret_id = state.secret_id;
    let import_id = state.import_id.expect("import");
    let instance_id = state.instance_id.expect("instance");
    let raw_revision_id = state.raw_revision_id.expect("raw revision");
    let attachment_id = state.attachment_id.expect("attachment");
    let run_id = state.run_id.expect("run");
    let authority = state.authority.as_ref().expect("authority");
    let raw_run_id = state.raw_run_id.expect("raw run");
    let raw_authority = state.raw_authority.as_ref().expect("raw authority");
    let first_version = state.first_version;
    let active_version = state.active_version.expect("active version");
    let expected_rotated_value = state
        .expected_rotated_value
        .as_ref()
        .expect("rotated value");
    let runtime = SecretRuntimeService::new(
        pool.clone(),
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("runtime fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let adapter = FakeBroker {
        observed: AtomicBool::new(false),
        expected_credential: SENTINEL.as_bytes().to_vec(),
    };

    sqlx::query(
        "UPDATE agent_updates
          SET state = 'rejected', final_decision = 'agent_rejected',
              completed_at = now(), updated_at = now()
          WHERE instance_id = $1 AND state = 'hook_running'",
    )
    .bind(instance_id.as_uuid())
    .execute(&pool)
    .await
    .expect("finish update fixture");
    sqlx::query(
        "UPDATE agent_instances
          SET active_revision_id = $2, state = 'active', run_gate_open = true
          WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(raw_revision_id.as_uuid())
    .execute(&pool)
    .await
    .expect("restore normal dispatch fixture");
    let concurrent_run_a =
        seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let concurrent_run_b =
        seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let resolve_a = service.resolve_for_dispatch(
        &target_manager,
        ResolveRunSecrets {
            command_key: key("concurrent-resolve-a", concurrent_run_a.as_uuid()),
            session_id: SecretRuntimeSessionId::new(),
            run_id: concurrent_run_a,
            instance_id,
            instance_revision_id: raw_revision_id,
            attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
            target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
            target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
            phase: ExecutionPhase::Normal,
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
        },
    );
    let resolve_b = service.resolve_for_dispatch(
        &target_manager,
        ResolveRunSecrets {
            command_key: key("concurrent-resolve-b", concurrent_run_b.as_uuid()),
            session_id: SecretRuntimeSessionId::new(),
            run_id: concurrent_run_b,
            instance_id,
            instance_revision_id: raw_revision_id,
            attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
            target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
            target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
            phase: ExecutionPhase::Normal,
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
        },
    );
    let (authority_a, authority_b) = tokio::join!(resolve_a, resolve_b);
    let authority_a = authority_a.expect("first concurrent dispatch");
    let authority_b = authority_b.expect("second concurrent dispatch");
    assert_eq!(authority_a.leases[0].version_id.as_uuid(), active_version);
    assert_eq!(authority_b.leases[0].version_id.as_uuid(), active_version);
    let rotated_raw = runtime
        .receive_raw(
            &authority_a.credential,
            concurrent_run_a,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await
        .expect("later dispatch uses rotated version");
    assert_eq!(
        rotated_raw.value.expose(),
        expected_rotated_value.as_bytes()
    );
    sqlx::query(
        "UPDATE secret_runtime_sessions
          SET issued_at = now() - interval '2 minutes',
              expires_at = now() - interval '1 minute'
          WHERE id = $1",
    )
    .bind(authority_b.session_id.as_uuid())
    .execute(&pool)
    .await
    .expect("expire exact runtime session");
    let expired_session = runtime
        .receive_raw(
            &authority_b.credential,
            concurrent_run_b,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    assert!(matches!(
        expired_session,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    sqlx::query(
        "UPDATE secret_leases SET status = 'revoked'
          WHERE id = $1",
    )
    .bind(authority_a.leases[0].lease_id.as_uuid())
    .execute(&pool)
    .await
    .expect("revoke exact lease");
    let stale_lease = runtime
        .receive_raw(
            &authority_a.credential,
            concurrent_run_a,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    match stale_lease {
        Err(SecretServiceError::RuntimeAuthenticationDenied | SecretServiceError::Unavailable) => {}
        Err(error) => panic!("unexpected stale lease error: {error}"),
        Ok(_) => panic!("revoked lease unexpectedly remained usable"),
    }
    let version_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_versions WHERE secret_id = $1")
            .bind(secret_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("version count");
    assert_eq!(version_count, 2);

    service
        .revoke_secret(&owner, key("revoke", secret_id.as_uuid()), secret_id)
        .await
        .expect("owner should revoke");
    let mut inspection = authz_postgres::begin_actor_transaction(&pool, &owner)
        .await
        .expect("historical inspection actor");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *inspection)
        .await
        .expect("historical application role");
    let historical_versions: Vec<Uuid> =
        sqlx::query_scalar("SELECT secret_version_id FROM inspect_run_https_uses($1, NULL, 200)")
            .bind(run_id.as_uuid())
            .fetch_all(&mut *inspection)
            .await
            .expect("inspect after rotation and revocation");
    assert!(historical_versions.len() >= 4);
    assert!(
        historical_versions
            .iter()
            .all(|version| *version == first_version.as_uuid())
    );
    inspection
        .rollback()
        .await
        .expect("close historical inspection");
    let broker_after_revocation = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("complete"),
                body: Vec::new(),
            },
            &adapter,
        )
        .await;
    assert!(matches!(
        broker_after_revocation,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    let raw_after_revocation = runtime
        .receive_raw(
            &raw_authority.credential,
            raw_run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    assert!(raw_after_revocation.is_err());
    let post_revoke_run = seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let before_resolution_denied = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("resolve-after-revoke", post_revoke_run.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: post_revoke_run,
                instance_id,
                instance_revision_id: raw_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await;
    assert!(before_resolution_denied.is_err());
    let revoked_runtime_state: (i64, i64, bool) = sqlx::query_as(
        "SELECT
              (SELECT count(*)::bigint FROM secret_runtime_sessions AS session
               JOIN secret_leases AS lease ON lease.session_id = session.id
               JOIN secret_versions AS version ON version.id = lease.secret_version_id
               WHERE version.secret_id = $1 AND session.status = 'active'),
              (SELECT count(*)::bigint FROM secret_leases AS lease
               JOIN secret_versions AS version ON version.id = lease.secret_version_id
               WHERE version.secret_id = $1 AND lease.status = 'active'),
              EXISTS(
                  SELECT 1 FROM secret_leases AS lease
                  JOIN secret_versions AS version ON version.id = lease.secret_version_id
                  WHERE version.secret_id = $1
                    AND lease.delivery_mode = 'raw'
                    AND lease.raw_material_observed
              )",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("revoked runtime authority");
    assert_eq!(revoked_runtime_state, (0, 0, true));
    let import_status: String =
        sqlx::query_scalar("SELECT status FROM secret_imports WHERE id = $1")
            .bind(import_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("import status");
    assert_eq!(import_status, "revoked");
    service
        .purge_secret(&owner, key("purge", secret_id.as_uuid()), secret_id)
        .await
        .expect("revoked secret without active leases should purge");
    let remaining_ciphertext: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM secret_versions
          WHERE secret_id = $1 AND (
             ciphertext IS NOT NULL OR wrapped_data_key IS NOT NULL
             OR data_nonce IS NOT NULL OR wrap_nonce IS NOT NULL
          )",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("purged encrypted material");
    assert_eq!(remaining_ciphertext, 0);
    let status: String = sqlx::query_scalar("SELECT status FROM secrets WHERE id = $1")
        .bind(secret_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("purged tombstone");
    assert_eq!(status, "purged");
}
