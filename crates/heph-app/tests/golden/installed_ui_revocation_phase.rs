use super::*;

pub async fn run_installed_ui_parent_revocation_phase(
    context: &InstalledUiBrowserContext<'_>,
    activated_context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) {
    let baseline =
        snapshot_installed_ui_parent_revocation(context, activated_context, control_dir, deadline)
            .await;
    let existing_children = [(baseline.child_id, baseline.parent_id)];
    write_installed_ui_control_marker(control_dir, "parent-revoke-permitted").await;
    wait_for_installed_ui_control_marker(control_dir, "parent-revoked-denied", deadline).await;
    assert_installed_ui_parent_revocation(context, activated_context, &baseline).await;
    write_installed_ui_control_marker(control_dir, "parent-revocation-verified").await;
    assert_installed_ui_relogin_child(
        context,
        activated_context,
        &baseline,
        &existing_children,
        control_dir,
        deadline,
    )
    .await;
    println!(
        "REAL_UI_INSTALLATION_PARENT_REVOCATION=1 parent_revoked=1 stale_cookie_denied=1 gateway_invocation_unchanged=1 fresh_parent_child=1"
    );
}

pub async fn run_installed_ui_removal_phase(
    context: &InstalledUiBrowserContext<'_>,
    activated_context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) {
    wait_for_installed_ui_control_marker(control_dir, "new-generation-verified", deadline).await;
    wait_for_installed_ui_control_marker(control_dir, "remove-ready", deadline).await;

    let removal_content_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE surface = 'content'
          ORDER BY occurred_at, id",
    )
    .fetch_all(context.pool)
    .await
    .expect("snapshot content audit IDs before removal");
    let removal_managed_audit_ids = installed_ui_audit_ids(activated_context).await;
    let removal_gateway_invocations = installed_ui_gateway_invocations(activated_context).await;
    let removed_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(context.pool)
        .await
        .expect("read installed UI removal audit boundary");

    let removed = cooking_builds::remove_installed_ui(
        context.running,
        context.rpc_token,
        activated_context.installed_uis.managed_ui,
    )
    .await
    .expect("remove managed installed UI through owner RPC");
    assert_eq!(
        removed.installation_id, activated_context.installed_uis.managed_ui.installation_id,
        "removal must retain the managed installation"
    );
    assert_eq!(
        removed.generation_id, activated_context.installed_uis.managed_ui.generation_id,
        "removal must retain the active generation"
    );
    let removed_state: (String, uuid::Uuid) = sqlx::query_as(
        "SELECT lifecycle, current_generation_id
           FROM ui_installations
          WHERE id = $1",
    )
    .bind(removed.installation_id)
    .fetch_one(context.pool)
    .await
    .expect("read removed installed UI lifecycle");
    assert_eq!(
        removed_state.0, "removed",
        "managed UI lifecycle after removal"
    );
    assert_eq!(
        removed_state.1, removed.generation_id,
        "removal must retain the removed generation"
    );
    write_installed_ui_control_marker(control_dir, "remove-complete").await;
    wait_for_installed_ui_control_marker(control_dir, "removed-host-denied", deadline).await;

    assert_installed_ui_stale_cookie_denial(
        activated_context,
        removed_at,
        &removal_content_audit_ids,
        UiRequestAuditReason::NotFound,
    )
    .await;
    assert_eq!(
        installed_ui_audit_ids(activated_context).await,
        removal_managed_audit_ids,
        "removed managed child must not create a managed-context audit"
    );
    assert_eq!(
        installed_ui_gateway_invocations(activated_context).await,
        removal_gateway_invocations,
        "removed managed child must not invoke the managed gateway"
    );
    wait_for_installed_ui_control_marker(control_dir, "removed-card-absent", deadline).await;
    println!(
        "REAL_UI_INSTALLATION_REMOVE_LIFECYCLE=1 stale_cookie_denied=1 gateway_invocation_unchanged=1 navigation_removed=1"
    );
}
