use super::*;

pub fn prepare_installed_ui_control_dir(fixture_path: &Path) -> PathBuf {
    let parent = fixture_path
        .parent()
        .expect("installed UI fixture must have a parent directory");
    let control_dir = parent.join("installed-ui-control");
    assert!(
        fs::symlink_metadata(&control_dir).is_err(),
        "installed UI control directory already exists"
    );
    fs::create_dir(&control_dir).expect("create installed UI control directory");
    fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o700))
        .expect("lock installed UI control directory");
    control_dir
}

pub async fn write_installed_ui_control_marker(control_dir: &Path, marker: &str) {
    assert!(INSTALLED_UI_CONTROL_MARKERS.contains(&marker));
    let path = control_dir.join(marker);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .await
        .expect("create installed UI control marker");
    file.write_all(b"ok\n")
        .await
        .expect("write installed UI control marker");
    file.sync_all()
        .await
        .expect("sync installed UI control marker");
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .expect("read installed UI control marker metadata");
    assert!(
        metadata.is_file(),
        "installed UI marker must be a regular file"
    );
    assert!(
        !metadata.file_type().is_symlink(),
        "installed UI marker must not be a symlink"
    );
}

pub async fn wait_for_installed_ui_control_marker(
    control_dir: &Path,
    marker: &str,
    deadline: tokio::time::Instant,
) {
    assert!(INSTALLED_UI_CONTROL_MARKERS.contains(&marker));
    let path = control_dir.join(marker);
    loop {
        if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await {
            assert!(
                metadata.is_file(),
                "installed UI marker must be a regular file"
            );
            assert!(
                !metadata.file_type().is_symlink(),
                "installed UI marker must not be a symlink"
            );
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for installed UI control marker {marker}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn assert_installed_ui_disable_did_not_reach_gateway(
    context: &InstalledUiBrowserContext<'_>,
    baseline_managed_audit_ids: &[uuid::Uuid],
    baseline_managed_invocations: i64,
) {
    let managed_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1 AND generation_id = $2
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .fetch_all(context.pool)
    .await
    .expect("read managed installed UI audits after disable");
    assert_eq!(
        managed_audit_ids, baseline_managed_audit_ids,
        "stale managed child must not create a managed-context audit"
    );
    let managed_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_one(context.pool)
    .await
    .expect("count managed gateway invocations after disable");
    assert_eq!(
        managed_invocations, baseline_managed_invocations,
        "stale managed child must not invoke the disabled gateway"
    );
}

pub async fn installed_ui_audit_ids(context: &InstalledUiBrowserContext<'_>) -> Vec<uuid::Uuid> {
    sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .fetch_all(context.pool)
    .await
    .expect("read all managed installation audits")
}

pub async fn installed_ui_gateway_invocations(context: &InstalledUiBrowserContext<'_>) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_one(context.pool)
    .await
    .expect("count managed gateway invocations")
}

pub async fn assert_installed_ui_stale_cookie_denial(
    context: &InstalledUiBrowserContext<'_>,
    disabled_at: OffsetDateTime,
    baseline_content_audit_ids: &[uuid::Uuid],
    expected_reason: UiRequestAuditReason,
) {
    let fresh_denials: Vec<InstalledUiDenialRow> = sqlx::query_as(
        "SELECT surface, decision, outcome, reason_code,
                actor_id, organization_id, installation_id, generation_id,
                child_session_id, gateway_id, gateway_revision_id
           FROM ui_request_audit_events
          WHERE occurred_at >= $1
            AND surface = 'content'
            AND NOT (id = ANY($2))
          ORDER BY occurred_at, id",
    )
    .bind(disabled_at)
    .bind(baseline_content_audit_ids)
    .fetch_all(context.pool)
    .await
    .expect("read stale managed child denial audit");
    assert_eq!(
        fresh_denials.len(),
        1,
        "stale managed child must emit exactly one fresh content denial"
    );
    let denial = &fresh_denials[0];
    assert_eq!(denial.0, "content");
    assert_eq!(denial.1, UiRequestAuditDecision::Denied.as_str());
    assert_eq!(denial.2, UiRequestAuditOutcome::NotAttempted.as_str());
    // The caller supplies the expected closed-vocabulary reason. Disabled or
    // removed hosts use not-found; revoked parents fail child authentication.
    assert_eq!(denial.3, expected_reason.as_str());
    assert!(denial.4.is_none(), "stale denial actor must be anonymous");
    assert!(
        denial.5.is_none(),
        "stale denial organization must be anonymous"
    );
    assert!(
        denial.6.is_none(),
        "stale denial installation must be anonymous"
    );
    assert!(
        denial.7.is_none(),
        "stale denial generation must be anonymous"
    );
    assert!(
        denial.8.is_none(),
        "stale denial child session must be anonymous"
    );
    assert!(denial.9.is_none(), "stale denial gateway must be anonymous");
    assert!(
        denial.10.is_none(),
        "stale denial gateway revision must be anonymous"
    );
}

pub async fn run_installed_ui_disable_phase(
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) -> (Vec<uuid::Uuid>, i64, Vec<uuid::Uuid>) {
    wait_for_installed_ui_control_marker(control_dir, "managed-ready", deadline).await;
    let baseline_managed_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1 AND generation_id = $2
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot managed installed UI audits before disable");
    let baseline_content_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE surface = 'content'
          ORDER BY occurred_at, id",
    )
    .fetch_all(context.pool)
    .await
    .expect("snapshot content audit IDs before disable");
    let baseline_managed_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_one(context.pool)
    .await
    .expect("snapshot managed gateway invocations before disable");
    let disabled_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(context.pool)
        .await
        .expect("read installed UI disable audit boundary");
    let disabled = cooking_builds::disable_installed_ui(
        context.running,
        context.rpc_token,
        context.installed_uis.managed_ui,
    )
    .await
    .expect("disable managed installed UI through owner RPC");
    assert_eq!(
        disabled.installation_id, context.installed_uis.managed_ui.installation_id,
        "disable must retain the managed installation"
    );
    let lifecycle: String = sqlx::query_scalar(
        "SELECT lifecycle
           FROM ui_installations
          WHERE id = $1",
    )
    .bind(disabled.installation_id)
    .fetch_one(context.pool)
    .await
    .expect("read disabled installed UI lifecycle");
    assert_eq!(lifecycle, "disabled", "managed UI lifecycle after disable");
    write_installed_ui_control_marker(control_dir, "disable-complete").await;
    wait_for_installed_ui_control_marker(control_dir, "stale-cookie-denied", deadline).await;

    assert_installed_ui_disable_did_not_reach_gateway(
        context,
        &baseline_managed_audit_ids,
        baseline_managed_invocations,
    )
    .await;
    assert_installed_ui_stale_cookie_denial(
        context,
        disabled_at,
        &baseline_content_audit_ids,
        UiRequestAuditReason::NotFound,
    )
    .await;

    let reactivation_content_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE surface = 'content'
          ORDER BY occurred_at, id",
    )
    .fetch_all(context.pool)
    .await
    .expect("snapshot content audit IDs before reactivation");
    (
        baseline_managed_audit_ids,
        baseline_managed_invocations,
        reactivation_content_audit_ids,
    )
}
