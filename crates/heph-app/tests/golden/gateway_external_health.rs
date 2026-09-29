use super::*;

pub async fn wait_for_external_daemon_health(daemon: &mut ExternalGoldenDaemon) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("external daemon health client");
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let Some(status) = daemon.child.try_wait().expect("poll external daemon child") {
                let log = std::fs::read_to_string(&daemon.log_path)
                    .unwrap_or_else(|error| format!("unable to read child log: {error}"));
                panic!(
                    "external hephaestusd exited before healthz: {status}; log={}\\n{}",
                    daemon.log_path.display(),
                    log
                );
            }
            if client
                .get(format!("{}/healthz", daemon.base_url))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "external hephaestusd health timeout; log={} ",
            daemon.log_path.display()
        )
    });
}

pub fn gateway_service_resource_paths(instance_id: uuid::Uuid) -> (PathBuf, PathBuf, PathBuf) {
    let vm_id = format!("gateway-service-{instance_id}");
    let runtime_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
            .expect("libkrun runtime root for service cleanup assertion"),
    );
    let cgroup_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
            .expect("libkrun cgroup root for service cleanup assertion"),
    );
    (
        runtime_root.join(&vm_id),
        cgroup_root.join(&vm_id),
        runtime_root
            .join("exact-runs")
            .join("gateway-services")
            .join(instance_id.to_string()),
    )
}

pub type GatewayServiceOwnership = (String, uuid::Uuid, i64, OffsetDateTime);

pub struct GatewayServiceRecoverySnapshot {
    pub old_state: String,
    pub old_host: String,
    pub old_owner: uuid::Uuid,
    pub old_fence: i64,
    pub old_lease: OffsetDateTime,
    pub old_cleaned_at: Option<OffsetDateTime>,
    pub active_revision: Option<uuid::Uuid>,
    pub candidate_count: i64,
    pub candidate_id: Option<uuid::Uuid>,
    pub candidate_state: Option<String>,
    pub candidate_host: Option<String>,
    pub candidate_owner: Option<uuid::Uuid>,
    pub candidate_fence: Option<i64>,
    pub candidate_created_at: Option<OffsetDateTime>,
    pub db_now: OffsetDateTime,
}

pub async fn read_gateway_service_ownership(
    pool: &sqlx::PgPool,
    instance_id: uuid::Uuid,
) -> GatewayServiceOwnership {
    sqlx::query_as(
        "SELECT owner_host_id, owner_uuid, fencing_token, lease_expires_at
           FROM gateway_service_instances
          WHERE id = $1",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .expect("read gateway service ownership")
}

pub async fn read_gateway_service_recovery_snapshot(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    old_instance_id: uuid::Uuid,
    recovery_started_at: OffsetDateTime,
) -> GatewayServiceRecoverySnapshot {
    let row = sqlx::query(
        r"
    SELECT old.state AS old_state,
           old.owner_host_id AS old_host,
           old.owner_uuid AS old_owner,
           old.fencing_token AS old_fence,
           old.lease_expires_at AS old_lease,
           old.cleaned_at AS old_cleaned_at,
           gateway.active_revision_id AS active_revision,
           (SELECT count(*)
              FROM gateway_service_instances historical
             WHERE historical.gateway_id = old.gateway_id
               AND historical.revision_id = old.revision_id
               AND historical.id <> old.id
               AND historical.created_at >= $4) AS candidate_count,
           candidate.id AS candidate_id,
           candidate.state AS candidate_state,
           candidate.owner_host_id AS candidate_host,
           candidate.owner_uuid AS candidate_owner,
           candidate.fencing_token AS candidate_fence,
           candidate.created_at AS candidate_created_at,
           clock_timestamp() AS db_now
      FROM gateway_service_instances old
      JOIN gateways gateway ON gateway.id = old.gateway_id
 LEFT JOIN LATERAL (
           SELECT id, state, owner_host_id, owner_uuid, fencing_token, created_at
             FROM gateway_service_instances current_instance
            WHERE current_instance.gateway_id = old.gateway_id
              AND current_instance.revision_id = old.revision_id
              AND current_instance.id <> old.id
              AND current_instance.created_at >= $4
              AND current_instance.state <> 'cleaned'
            ORDER BY created_at DESC, id DESC
            LIMIT 1
      ) candidate ON TRUE
     WHERE old.id = $1
       AND old.gateway_id = $2
       AND old.revision_id = $3
",
    )
    .bind(old_instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .bind(recovery_started_at)
    .fetch_one(pool)
    .await
    .expect("read coherent boot recovery snapshot");
    GatewayServiceRecoverySnapshot {
        old_state: row.get("old_state"),
        old_host: row.get("old_host"),
        old_owner: row.get("old_owner"),
        old_fence: row.get("old_fence"),
        old_lease: row.get("old_lease"),
        old_cleaned_at: row.get("old_cleaned_at"),
        active_revision: row.get("active_revision"),
        candidate_count: row.get("candidate_count"),
        candidate_id: row.get("candidate_id"),
        candidate_state: row.get("candidate_state"),
        candidate_host: row.get("candidate_host"),
        candidate_owner: row.get("candidate_owner"),
        candidate_fence: row.get("candidate_fence"),
        candidate_created_at: row.get("candidate_created_at"),
        db_now: row.get("db_now"),
    }
}

pub async fn wait_for_gateway_service_cleaned(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    instance_id: uuid::Uuid,
) {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM gateway_service_instances
                   WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
            )
            .bind(instance_id)
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .fetch_optional(pool)
            .await
            .expect("read gateway service cleanup state");
            if state.as_deref() == Some("cleaned") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("gateway service reaches durable Cleaned state");
}

pub async fn wait_for_gateway_service_boot_replacement(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    old_instance_id: uuid::Uuid,
    old_ownership: GatewayServiceOwnership,
    old_paths: (PathBuf, PathBuf, PathBuf),
    recovery_started_at: OffsetDateTime,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut observed_unexpired_empty = false;
        loop {
            let snapshot = read_gateway_service_recovery_snapshot(
                pool,
                fixture,
                old_instance_id,
                recovery_started_at,
            )
            .await;

            if snapshot.db_now < old_ownership.3 {
                assert_eq!(
                    snapshot.candidate_count, 0,
                    "no replacement attempt may be admitted while the old lease is live"
                );
                observed_unexpired_empty = true;
            }
            if let Some(candidate_id) = snapshot.candidate_id {
                assert!(
                    snapshot.db_now >= old_ownership.3,
                    "a replacement service must not be admitted before the old lease expires"
                );
                assert_eq!(
                    snapshot.old_state, "cleaned",
                    "old service must be durably cleaned before replacement admission"
                );
                assert!(
                    snapshot.old_lease >= old_ownership.3,
                    "recovery must renew the fenced old claim from the original lease"
                );
                assert_eq!(snapshot.old_host, old_ownership.0);
                assert_eq!(
                    snapshot.candidate_host.as_deref(),
                    Some(snapshot.old_host.as_str())
                );
                assert_ne!(snapshot.old_owner, old_ownership.1);
                assert!(snapshot.old_fence > old_ownership.2);
                assert_eq!(snapshot.candidate_owner, Some(snapshot.old_owner));
                assert_ne!(snapshot.candidate_owner, Some(old_ownership.1));
                assert!(snapshot.candidate_fence.is_some_and(|fence| fence > 0));
                assert!(
                    snapshot
                        .candidate_created_at
                        .expect("candidate creation timestamp")
                        >= old_ownership.3,
                    "replacement creation must follow old lease expiry"
                );
                assert!(
                    snapshot
                        .candidate_created_at
                        .expect("candidate creation timestamp")
                        >= snapshot.old_cleaned_at.expect("old cleanup timestamp"),
                    "replacement creation must follow old durable cleanup"
                );
                assert!(
                    !old_paths.0.exists(),
                    "old VM runtime must be gone before Ready"
                );
                assert!(
                    !old_paths.1.exists(),
                    "old cgroup must be gone before Ready"
                );
                assert!(
                    !old_paths.2.exists(),
                    "old materializer must be gone before Ready"
                );
                if snapshot.candidate_state.as_deref() == Some("ready")
                    && snapshot.active_revision == Some(fixture.revision_id)
                {
                    assert!(
                        observed_unexpired_empty,
                        "must observe an unexpired window with no replacement rows"
                    );
                    return candidate_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("daemon boot recovery replaces the abandoned service")
}
