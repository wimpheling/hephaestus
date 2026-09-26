// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Error returned when source preparation, build observation, or publication
/// crosses a production boundary unsuccessfully.
pub type BuildError = Box<dyn Error + Send + Sync>;

pub(crate) type CookingGatewayServiceInstance =
    (Uuid, i64, String, Option<String>, Option<i32>, Option<i32>);

/// Authentication material needed by the Git and Connect boundaries.
///
/// Git uses the OIDC bearer assertion accepted by the smart HTTP endpoint;
/// the RPC token factory creates a short-lived method-audience-bound mediator
/// assertion for each call.  The factory is required because the mediator
/// rejects assertions whose audience is a different RPC procedure.
#[derive(Clone, Copy)]
pub(crate) struct CookingIdentity<'a> {
    /// Actor used when creating the repositories and persisted build rows.
    pub actor: &'a AuthenticatedIdentity,
    /// Bearer token accepted by Git smart HTTP.
    pub git_token: &'a str,
    /// Creates a fresh bearer token for the exact Connect procedure path.
    pub rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
}

/// Existing fixture resources required by [`build_and_publish`].
pub(crate) struct CookingBuildContext<'a> {
    /// Application database used only to observe receive/build state.
    pub pool: &'a PgPool,
    /// Running daemon exposing Git HTTP and Connect RPC.
    pub running: &'a RunningHephaestus,
    /// Per-test root under which temporary source checkouts are created.
    pub root: &'a Path,
    /// Selected canonical source root (the same override used by the runner).
    pub source_root: &'a Path,
    /// Project receiving the two cooking source repositories.
    pub project_id: ProjectId,
    /// Trusted repository factory from the already-configured test fixture.
    pub repositories: &'a PgForgeRepository,
    /// Authenticated actor and boundary tokens.
    pub identity: CookingIdentity<'a>,
    /// Bound for each asynchronous build/release wait.
    pub timeout: Duration,
}

/// Waits until all asynchronous build work belonging to the cooking project
/// has reached a durable terminal state before an intentional daemon restart.
///
/// The OCI materialization queue has no project column, so it is narrowed by
/// the exact cooking worker name and an image reference owned by this project.
/// A failed terminal row is reported immediately; silently restarting over a
/// failed build would make the later cooking assertions misleading.
pub(crate) async fn wait_for_cooking_build_quiescence(
    pool: &PgPool,
    project_id: ProjectId,
    materialization_worker_name: &str,
    timeout: Duration,
) {
    assert!(!materialization_worker_name.trim().is_empty());
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let counts =
            cooking_build_queue_counts(pool, project_id, materialization_worker_name).await;
        if counts.build_failed != 0 {
            let details = cooking_build_failure_details(pool, project_id).await;
            assert_eq!(
                counts.build_failed, 0,
                "cooking build queue contains failed or cancelled terminal work:\n{details}"
            );
        }
        assert_eq!(
            counts.production_failed, 0,
            "cooking OCI production queue contains failed terminal work"
        );
        assert_eq!(
            counts.definition_failed, 0,
            "cooking OCI definition contains failed terminal work"
        );
        assert_eq!(
            counts.materialization_failed, 0,
            "cooking OCI materialization queue contains failed terminal work"
        );
        if counts.is_quiescent() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cooking build queues did not quiesce: build_pending={}, \
             production_pending={}, definition_pending={}, materialization_pending={}",
            counts.build_pending,
            counts.production_pending,
            counts.definition_pending,
            counts.materialization_pending
        );
        sleep(Duration::from_millis(250)).await;
    }
}

/// Returns bounded, credential-redacted details for failed build rows before
/// the queue assertion aborts the scenario. The normal Build RPC exposes the
/// same fields; this project-wide queue observer uses the fixture pool to
/// retain guest stderr before teardown removes the disposable database.
pub(crate) async fn cooking_build_failure_details(pool: &PgPool, project_id: ProjectId) -> String {
    let rows: Vec<CookingBuildFailure> = sqlx::query_as(
        "SELECT build.id, build.source_commit, build.state,
                execution.failure_code, execution.exit_code,
                execution.exit_signal, execution.logs
           FROM build_requests build
           JOIN repositories repository ON repository.id = build.repository_id
           LEFT JOIN build_executions execution
             ON execution.build_request_id = build.id
          WHERE repository.project_id = $1
            AND build.state IN ('failed', 'cancelled')
          ORDER BY build.created_at, build.id
          LIMIT 16",
    )
    .bind(project_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("inspect failed cooking build details");

    rows.into_iter()
        .map(|failure| {
            let logs = failure
                .logs
                .and_then(|value| value.as_array().cloned())
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter_map(|entry| {
                            let stream = entry.get("stream")?.as_str()?;
                            let text = entry.get("text")?.as_str()?;
                            Some(format!("[{stream}] {}", redact_build_log(text)))
                        })
                        .collect::<Vec<_>>()
                        .join("\\n")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| String::from("<no retained guest logs>"));
            format!(
                "build_id={} state={} source_commit={} failure_code={:?} \
                     exit_code={:?} exit_signal={:?} logs={}",
                failure.id,
                failure.state,
                failure.source_commit,
                failure.failure_code,
                failure.exit_code,
                failure.exit_signal,
                logs
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(sqlx::FromRow)]
pub(crate) struct CookingBuildFailure {
    id: Uuid,
    source_commit: String,
    state: String,
    failure_code: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    logs: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct CookingBuildQueueCounts {
    build_pending: i64,
    build_failed: i64,
    production_pending: i64,
    production_failed: i64,
    definition_pending: i64,
    definition_failed: i64,
    materialization_pending: i64,
    materialization_failed: i64,
}

impl CookingBuildQueueCounts {
    const fn is_quiescent(&self) -> bool {
        self.build_pending == 0
            && self.production_pending == 0
            && self.definition_pending == 0
            && self.materialization_pending == 0
    }
}

pub(crate) async fn cooking_build_queue_counts(
    pool: &PgPool,
    project_id: ProjectId,
    materialization_worker_name: &str,
) -> CookingBuildQueueCounts {
    let counts: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
                (SELECT count(DISTINCT build.id)
                   FROM build_requests build
                   JOIN repositories repository ON repository.id = build.repository_id
                  WHERE repository.project_id = $1
                    AND build.state IN ('queued', 'running', 'importing')),
                (SELECT count(DISTINCT build.id)
                   FROM build_requests build
                   JOIN repositories repository ON repository.id = build.repository_id
                  WHERE repository.project_id = $1
                    AND build.state IN ('failed', 'cancelled')),
                (SELECT count(DISTINCT production.id)
                   FROM repository_oci_image_production_jobs production
                   JOIN repository_oci_image_definitions definition
                     ON definition.id = production.definition_id
                  WHERE definition.project_id = $1
                    AND production.state IN ('queued', 'claimed')),
                (SELECT count(DISTINCT production.id)
                   FROM repository_oci_image_production_jobs production
                   JOIN repository_oci_image_definitions definition
                     ON definition.id = production.definition_id
                  WHERE definition.project_id = $1
                    AND production.state = 'failed'),
                (SELECT count(DISTINCT definition.id)
                   FROM repository_oci_image_definitions definition
                  WHERE definition.project_id = $1
                    AND definition.status = 'producing'),
                (SELECT count(DISTINCT definition.id)
                   FROM repository_oci_image_definitions definition
                  WHERE definition.project_id = $1
                    AND definition.status = 'failed'),
                (SELECT count(DISTINCT materialization.id)
                   FROM oci_image_materialization_jobs materialization
                  WHERE materialization.worker_name = $2
                    AND materialization.state IN ('queued', 'claimed')
                    AND EXISTS (
                        SELECT 1
                          FROM repository_oci_image_definitions definition
                         WHERE definition.project_id = $1
                           AND definition.image_reference = materialization.image_reference
                    )),
                (SELECT count(DISTINCT materialization.id)
                   FROM oci_image_materialization_jobs materialization
                  WHERE materialization.worker_name = $2
                    AND materialization.state = 'failed'
                    AND EXISTS (
                        SELECT 1
                          FROM repository_oci_image_definitions definition
                         WHERE definition.project_id = $1
                           AND definition.image_reference = materialization.image_reference
                    ))",
    )
    .bind(project_id.as_uuid())
    .bind(materialization_worker_name)
    .fetch_one(pool)
    .await
    .expect("inspect cooking build queues");
    CookingBuildQueueCounts {
        build_pending: counts.0,
        build_failed: counts.1,
        production_pending: counts.2,
        production_failed: counts.3,
        definition_pending: counts.4,
        definition_failed: counts.5,
        materialization_pending: counts.6,
        materialization_failed: counts.7,
    }
}
