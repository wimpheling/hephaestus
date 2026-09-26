use super::*;

pub async fn run_installed_ui_reactivation_phase(
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
    baseline_managed_audit_ids: &[uuid::Uuid],
    baseline_managed_invocations: i64,
    reactivation_content_audit_ids: &[uuid::Uuid],
) -> cooking_builds::InstalledCookingUi {
    let reactivated_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(context.pool)
        .await
        .expect("read installed UI reactivation audit boundary");
    let reactivated = cooking_builds::activate_installed_ui(
        context.running,
        context.rpc_token,
        context.installed_uis.managed_ui,
        context.installed_uis.managed_release_id,
    )
    .await
    .expect("reactivate managed installed UI through owner RPC");
    let current_generation: uuid::Uuid = sqlx::query_scalar(
        "SELECT current_generation_id
           FROM ui_installations
          WHERE id = $1 AND lifecycle = 'enabled'",
    )
    .bind(reactivated.installation_id)
    .fetch_one(context.pool)
    .await
    .expect("read reactivated managed UI generation");
    assert_eq!(
        current_generation, reactivated.generation_id,
        "reactivation must publish its fresh enabled generation"
    );
    write_installed_ui_control_marker(control_dir, "reactivate-complete").await;
    wait_for_installed_ui_control_marker(
        control_dir,
        "old-generation-denied-after-reactivate",
        deadline,
    )
    .await;
    assert_installed_ui_disable_did_not_reach_gateway(
        context,
        baseline_managed_audit_ids,
        baseline_managed_invocations,
    )
    .await;
    assert_installed_ui_stale_cookie_denial(
        context,
        reactivated_at,
        reactivation_content_audit_ids,
        UiRequestAuditReason::NotFound,
    )
    .await;
    write_installed_ui_control_marker(control_dir, "old-generation-denial-verified").await;
    wait_for_installed_ui_control_marker(control_dir, "new-generation-ready", deadline).await;
    reactivated
}

pub async fn installed_ui_current_generation_children(
    context: &InstalledUiBrowserContext<'_>,
) -> Vec<(uuid::Uuid, uuid::Uuid)> {
    sqlx::query_as(
        "SELECT id, parent_session_id
           FROM ui_browser_sessions
          WHERE installation_id = $1
            AND generation_id = $2
          ORDER BY issued_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .fetch_all(context.pool)
    .await
    .expect("read current-generation installed UI children")
}

pub type InstalledUiServiceEvidence = (String, Option<String>, Option<i32>, Option<i32>);

pub fn installed_ui_service_resource_paths(
    instance_id: uuid::Uuid,
    materializer_root: &Path,
) -> (PathBuf, PathBuf, PathBuf) {
    let vm_id = format!("gateway-service-{instance_id}");
    let runtime_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT")
            .expect("libkrun runtime root for installed UI service replacement"),
    );
    let cgroup_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
            .expect("libkrun cgroup root for installed UI service replacement"),
    );
    (
        runtime_root.join(&vm_id),
        cgroup_root.join(&vm_id),
        materializer_root
            .join("run-runtime")
            .join("gateway-services")
            .join(instance_id.to_string()),
    )
}

pub fn kill_exact_installed_ui_service_guest(instance_id: uuid::Uuid) -> PathBuf {
    let cgroup_root = PathBuf::from(
        env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT")
            .expect("delegated libkrun cgroup root for installed UI replacement"),
    );
    let vm_id = format!("gateway-service-{instance_id}");
    let cgroup_path = cgroup_root.join(&vm_id);
    let root_metadata =
        fs::symlink_metadata(&cgroup_root).expect("read delegated libkrun cgroup root metadata");
    assert!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "installed UI replacement requires a real delegated cgroup root"
    );
    let cgroup_metadata = fs::symlink_metadata(&cgroup_path)
        .expect("read exact installed UI service cgroup metadata");
    assert!(
        cgroup_metadata.is_dir() && !cgroup_metadata.file_type().is_symlink(),
        "installed UI replacement target must be the exact service cgroup directory"
    );
    let kill_path = cgroup_path.join("cgroup.kill");
    let kill_metadata = fs::symlink_metadata(&kill_path)
        .expect("read exact installed UI service cgroup.kill metadata");
    assert!(
        !kill_metadata.file_type().is_symlink(),
        "installed UI replacement must not follow a cgroup.kill symlink"
    );
    fs::write(&kill_path, b"1\n").expect("SIGKILL exact installed UI service cgroup");
    cgroup_path
}

pub async fn wait_for_installed_ui_service_replacement(
    context: &InstalledUiBrowserContext<'_>,
    old_instance_id: uuid::Uuid,
    old_instance_ids: &[uuid::Uuid],
    old_paths: &(PathBuf, PathBuf, PathBuf),
    deadline: tokio::time::Instant,
) -> (uuid::Uuid, (PathBuf, PathBuf, PathBuf)) {
    let gateway_id = context.installed_uis.managed_gateway_id;
    let revision_id = context.installed_uis.managed_gateway_revision_id;
    loop {
        let old: Option<InstalledUiServiceEvidence> = sqlx::query_as(
            "SELECT state, failure_code, exit_code, exit_signal
               FROM gateway_service_instances
              WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
        )
        .bind(old_instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_optional(context.pool)
        .await
        .expect("read replaced installed UI service evidence");
        let rows: Vec<(uuid::Uuid, String, uuid::Uuid)> = sqlx::query_as(
            "SELECT id, state, revision_id
               FROM gateway_service_instances
              WHERE gateway_id = $1 AND revision_id = $2
              ORDER BY created_at, id",
        )
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_all(context.pool)
        .await
        .expect("read installed UI service replacement rows");
        let fresh_ready: Vec<(uuid::Uuid, String, uuid::Uuid)> = rows
            .iter()
            .filter(|row| !old_instance_ids.contains(&row.0) && row.1 == "ready")
            .cloned()
            .collect();
        let active_revision: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                .bind(gateway_id)
                .fetch_one(context.pool)
                .await
                .expect("read installed UI active gateway revision");
        if let Some((new_instance_id, state, new_revision_id)) = fresh_ready.first()
            && fresh_ready.len() == 1
            && rows.iter().filter(|row| row.1 != "cleaned").count() == 1
            && active_revision == Some(revision_id)
            && *new_revision_id == revision_id
            && old
                == Some((
                    String::from("cleaned"),
                    Some(String::from("unexpected_exit")),
                    None,
                    Some(9),
                ))
            && !old_paths.0.exists()
            && !old_paths.1.exists()
            && !old_paths.2.exists()
        {
            let new_paths = installed_ui_service_resource_paths(
                *new_instance_id,
                context.service_materializer_root,
            );
            if new_paths.0.is_dir() && new_paths.1.is_dir() && new_paths.2.is_dir() {
                assert_eq!(state, "ready");
                return (*new_instance_id, new_paths);
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out replacing the installed UI service guest"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub type InstalledUiReplacementAuditRow = (
    uuid::Uuid,
    uuid::Uuid,
    String,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    String,
    String,
    String,
    String,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    Option<uuid::Uuid>,
);
