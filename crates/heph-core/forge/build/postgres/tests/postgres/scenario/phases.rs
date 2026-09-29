//! Named phases of the isolated build durability scenario.

use build_orchestrator::BuildExecutionError;
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseArtifactId, ReleaseId};
use std::{fs, os::unix::fs::PermissionsExt, sync::atomic::Ordering};
use uuid::Uuid;
use vm_trait::RootFilesystem;

use super::super::support::copy_build_request;
use super::Scenario;

pub async fn initial(scenario: &Scenario) {
    let release_id = initial_draft(scenario).await;
    verify_draft(scenario, release_id).await;
}

async fn initial_draft(scenario: &Scenario) -> ReleaseId {
    assert!(matches!(
        scenario.executor.execute(scenario.build_id).await,
        Err(BuildExecutionError::ImageUnavailable)
    ));
    assert_eq!(scenario.provisions.load(Ordering::SeqCst), 0);
    let queued_state: (String, Option<String>) = sqlx::query_as(
        "SELECT request.state, execution.state
           FROM build_requests AS request
           LEFT JOIN build_executions AS execution
             ON execution.build_request_id = request.id
          WHERE request.id = $1",
    )
    .bind(scenario.build_id.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("durable request remains queued while image catches up");
    assert_eq!(queued_state, (String::from("queued"), None));
    scenario
        .image_filesystems
        .write()
        .expect("image cache lock")
        .insert(
            String::from(
                "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            RootFilesystem::Directory {
                host_path: scenario.root.join("root-image"),
            },
        );
    let first = scenario
        .executor
        .execute(scenario.build_id)
        .await
        .expect("isolated build");
    let duplicate = scenario
        .executor
        .execute(scenario.build_id)
        .await
        .expect("idempotent replay");
    assert_eq!(first, duplicate);
    assert_eq!(first.artifact_count, 1);
    assert_eq!(scenario.provisions.load(Ordering::SeqCst), 1);
    scenario
        .image_filesystems
        .write()
        .expect("image cache lock")
        .remove("build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(matches!(
        scenario.executor.verify(scenario.build_id).await,
        Err(BuildExecutionError::ImageUnavailable)
    ));
    let verification_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM build_verifications WHERE build_request_id = $1")
            .bind(scenario.build_id.as_uuid())
            .fetch_one(&scenario.pool)
            .await
            .expect("verification row count");
    assert_eq!(verification_rows, 0);
    scenario
        .image_filesystems
        .write()
        .expect("image cache lock")
        .insert(
            String::from(
                "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            RootFilesystem::Directory {
                host_path: scenario.root.join("root-image"),
            },
        );
    first.release_id
}

async fn verify_draft(scenario: &Scenario, release_id: ReleaseId) {
    let state: (String, String, i32, i32) = sqlx::query_as(
        "SELECT request.state, execution.state, execution.exit_code,
                jsonb_array_length(execution.logs)
         FROM build_requests AS request
         JOIN build_executions AS execution
           ON execution.build_request_id = request.id
         WHERE request.id = $1",
    )
    .bind(scenario.build_id.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("durable build result");
    assert_eq!(
        state,
        (String::from("succeeded"), String::from("drafted"), 0, 1)
    );
    let release_state: String = sqlx::query_scalar("SELECT state FROM releases WHERE id = $1")
        .bind(release_id.as_uuid())
        .fetch_one(&scenario.pool)
        .await
        .expect("draft release");
    assert_eq!(release_state, "draft");
    let (path, mode, storage_key): (String, i32, Uuid) = sqlx::query_as(
        "SELECT path, mode, storage_key
         FROM release_artifacts WHERE release_id = $1",
    )
    .bind(release_id.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("release artifact");
    assert_eq!((path.as_str(), mode), ("bin/agent", 0o555));
    assert_eq!(
        fs::metadata(
            scenario
                .artifact_root
                .join(storage_key.simple().to_string())
        )
        .expect("canonical object")
        .permissions()
        .mode()
            & 0o777,
        0o400
    );
}

pub async fn recovery(scenario: &Scenario) {
    let imported_build = BuildRequestId::new();
    copy_build_request(
        &scenario.pool,
        scenario.build_id,
        imported_build,
        [7_u8; 32],
    )
    .await;
    let recovered_output = scenario.root.join("recovered-output");
    fs::create_dir_all(recovered_output.join("bin")).expect("recovered output");
    let recovered_executable = recovered_output.join("bin/agent");
    fs::write(&recovered_executable, b"durably imported executable").expect("recovered executable");
    fs::set_permissions(&recovered_executable, fs::Permissions::from_mode(0o500))
        .expect("recovered executable mode");
    fs::set_permissions(&recovered_output, fs::Permissions::from_mode(0o500))
        .expect("sealed recovered output");
    let imported = scenario
        .artifact_store
        .import_for(imported_build.as_uuid(), &recovered_output)
        .expect("pre-crash artifact import");
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let artifact_id = ReleaseArtifactId::new();
    let artifact = imported.first().expect("one imported artifact");
    let manifest = serde_json::json!([{
        "id": artifact_id,
        "path": artifact.path,
        "kind": "executable",
        "mode": artifact.mode,
        "content_hash": artifact.content_hash,
        "size_bytes": artifact.size_bytes,
        "media_type": "application/x-hephaestus-test",
        "storage_key": artifact.storage_key,
    }]);
    sqlx::query("UPDATE build_requests SET state = 'importing' WHERE id = $1")
        .bind(imported_build.as_uuid())
        .execute(&scenario.pool)
        .await
        .expect("importing request");
    sqlx::query(
        "INSERT INTO build_executions
         (build_request_id, vm_id, release_id, release_agent_id,
          release_version, state, exit_code, artifact_manifest,
          sealed_at, imported_at)
         VALUES ($1, $2, $3, $4, $5, 'imported', 0, $6, now(), now())",
    )
    .bind(imported_build.as_uuid())
    .bind(format!("build-{imported_build}"))
    .bind(release_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(format!(
        "build-{}",
        &imported_build.as_uuid().simple().to_string()[..16]
    ))
    .bind(manifest)
    .execute(&scenario.pool)
    .await
    .expect("durable imported boundary");
    let recovered = scenario
        .executor
        .execute(imported_build)
        .await
        .expect("resume imported build without VM");
    assert_eq!(recovered.release_id, release_id);
    assert_eq!(recovered.release_agent_id, release_agent_id);
    assert_eq!(scenario.provisions.load(Ordering::SeqCst), 1);
    let recovered_artifact: (Uuid, Uuid) =
        sqlx::query_as("SELECT id, storage_key FROM release_artifacts WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&scenario.pool)
            .await
            .expect("recovered release artifact");
    assert_eq!(
        recovered_artifact,
        (artifact_id.as_uuid(), artifact.storage_key)
    );
}

pub async fn failure_and_authorization(scenario: &Scenario) {
    failed_execution(scenario).await;
    authorization_denial(scenario).await;
}

async fn failed_execution(scenario: &Scenario) {
    let failed_build = BuildRequestId::new();
    copy_build_request(&scenario.pool, scenario.build_id, failed_build, [6_u8; 32]).await;
    scenario.fail_next_provision.store(true, Ordering::SeqCst);
    assert!(matches!(
        scenario.executor.execute(failed_build).await,
        Err(BuildExecutionError::Vm)
    ));
    let failed_state: (String, String, String, serde_json::Value) = sqlx::query_as(
        "SELECT request.state, execution.state, execution.failure_code, request.diagnostics
         FROM build_requests AS request
         JOIN build_executions AS execution
           ON execution.build_request_id = request.id
         WHERE request.id = $1",
    )
    .bind(failed_build.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("durable build failure");
    assert_eq!(
        failed_state,
        (
            String::from("failed"),
            String::from("failed"),
            String::from("vm_provision"),
            serde_json::json!([{"code": "vm_provision"}]),
        )
    );

    let failed_event: (String, String, String, Uuid, String) = sqlx::query_as(
        "SELECT event.event_type, event.change_kind, event.safe_state,
                event.related_id_one, outbox.subject
         FROM application_events AS event
         JOIN product_event_outbox AS outbox ON outbox.event_id = event.id
         WHERE event.aggregate_type = 'build' AND event.aggregate_id = $1
           AND event.event_type = 'build.changed'
           AND event.change_kind = 'state_changed'
           AND event.safe_state = 'failed'
         ORDER BY event.cursor DESC
         LIMIT 1",
    )
    .bind(failed_build.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("canonical transactional build failure event");
    assert_eq!(
        failed_event,
        (
            String::from("build.changed"),
            String::from("state_changed"),
            String::from("failed"),
            scenario.repository_id,
            String::from("hephaestus.product.event.v1"),
        )
    );

    scenario
        .image_filesystems
        .write()
        .expect("image cache lock")
        .remove("build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(matches!(
        scenario.executor.retry(failed_build).await,
        Err(BuildExecutionError::ImageUnavailable)
    ));
    let still_failed: (String, String, String) = sqlx::query_as(
        "SELECT request.state, execution.state, execution.failure_code
           FROM build_requests AS request
           JOIN build_executions AS execution
             ON execution.build_request_id = request.id
          WHERE request.id = $1",
    )
    .bind(failed_build.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("terminal failure remains durable while image is unavailable");
    assert_eq!(
        still_failed,
        (
            String::from("failed"),
            String::from("failed"),
            String::from("vm_provision"),
        )
    );
    scenario
        .image_filesystems
        .write()
        .expect("image cache lock")
        .insert(
            String::from(
                "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            RootFilesystem::Directory {
                host_path: scenario.root.join("root-image"),
            },
        );
}

async fn authorization_denial(scenario: &Scenario) {
    let denied_build = BuildRequestId::new();
    copy_build_request(&scenario.pool, scenario.build_id, denied_build, [8_u8; 32]).await;
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE project_id = (
             SELECT repository.project_id
             FROM build_requests request
             JOIN repositories repository ON repository.id = request.repository_id
             WHERE request.id = $1
         )",
    )
    .bind(denied_build.as_uuid())
    .execute(&scenario.pool)
    .await
    .expect("revoke build authority");
    assert!(matches!(
        scenario.executor.execute(denied_build).await,
        Err(BuildExecutionError::Unauthorized)
    ));
    assert_eq!(scenario.provisions.load(Ordering::SeqCst), 2);
    let denied_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE object_type = 'build' AND object_id = $1
           AND permission = 'can_execute' AND decision = 'deny'",
    )
    .bind(denied_build.as_uuid())
    .fetch_one(&scenario.pool)
    .await
    .expect("denial audit");
    assert_eq!(denied_audits, 1);
}
