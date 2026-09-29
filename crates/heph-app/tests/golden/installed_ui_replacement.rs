use super::*;

// This keeps the replacement proof's snapshot, kill, barrier, and exact
// correlation assertions in one ordered lifecycle boundary.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run_installed_ui_live_service_replacement(
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) {
    let replacement_deadline = deadline.min(
        tokio::time::Instant::now()
            .checked_add(Duration::from_secs(90))
            .expect("installed UI replacement deadline"),
    );
    wait_for_installed_ui_control_marker(
        control_dir,
        "managed-restart-ready",
        replacement_deadline,
    )
    .await;
    let children_before = installed_ui_current_generation_children(context).await;
    assert_eq!(
        children_before.len(),
        1,
        "live service replacement requires exactly one existing UI child"
    );
    let old_instance_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY created_at, id",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .bind(context.installed_uis.managed_gateway_revision_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot installed UI service instance IDs");
    let old_ready: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT instance.id
           FROM gateway_service_instances AS instance
           JOIN gateways AS gateway ON gateway.id = instance.gateway_id
          WHERE instance.gateway_id = $1
            AND instance.revision_id = $2
            AND instance.state = 'ready'
            AND gateway.active_revision_id = $2
          ORDER BY instance.created_at, instance.id",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .bind(context.installed_uis.managed_gateway_revision_id)
    .fetch_all(context.pool)
    .await
    .expect("read active installed UI service instance");
    assert_eq!(
        old_ready.len(),
        1,
        "installed UI service must have exactly one active ready instance before replacement"
    );
    let old_instance_id = old_ready[0];
    let old_paths =
        installed_ui_service_resource_paths(old_instance_id, context.service_materializer_root);
    assert!(old_paths.0.is_dir() && old_paths.1.is_dir() && old_paths.2.is_dir());
    let baseline_audits: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1 AND generation_id = $2
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot installed UI audits before service replacement");
    let baseline_invocations: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM gateway_invocations
          WHERE gateway_id = $1
          ORDER BY accepted_at, id",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot all installed UI gateway invocations before replacement");

    let _old_cgroup = kill_exact_installed_ui_service_guest(old_instance_id);
    let (new_instance_id, new_paths) = wait_for_installed_ui_service_replacement(
        context,
        old_instance_id,
        &old_instance_ids,
        &old_paths,
        replacement_deadline,
    )
    .await;
    assert_ne!(old_instance_id, new_instance_id);
    write_installed_ui_control_marker(control_dir, "managed-restart-complete").await;
    wait_for_installed_ui_control_marker(
        control_dir,
        "managed-restart-verified",
        replacement_deadline,
    )
    .await;

    let children_after = installed_ui_current_generation_children(context).await;
    assert_eq!(
        children_after, children_before,
        "service replacement must preserve the existing browser child exactly"
    );
    let fresh_rows: Vec<InstalledUiReplacementAuditRow> = sqlx::query_as(
        "SELECT audit.id, audit.request_id, audit.surface,
                audit.actor_id, audit.organization_id, audit.installation_id,
                audit.generation_id, audit.child_session_id,
                audit.decision, audit.outcome, audit.reason_code,
                invocation.outcome, invocation.id, invocation.gateway_id,
                invocation.gateway_revision_id, invocation.service_instance_id
           FROM ui_request_audit_events AS audit
           JOIN gateway_invocations AS invocation
             ON invocation.request_id = audit.request_id
          WHERE audit.installation_id = $1
            AND audit.generation_id = $2
            AND audit.child_session_id = $3
            AND audit.surface IN ('managed', 'api')
            AND audit.decision = 'allowed'
            AND audit.outcome = 'succeeded'
            AND audit.reason_code = 'none'
            AND NOT (audit.id = ANY($4))
          ORDER BY audit.occurred_at, audit.id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .bind(children_before[0].0)
    .bind(&baseline_audits)
    .fetch_all(context.pool)
    .await
    .expect("read installed UI audits correlated to replacement service");
    let fresh_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1
            AND generation_id = $2
            AND NOT (id = ANY($3))
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .bind(&baseline_audits)
    .fetch_all(context.pool)
    .await
    .expect("read all fresh installed UI replacement audits");
    assert_eq!(
        fresh_rows.len(),
        2,
        "replacement must produce exactly two fresh UI audits"
    );
    let mut surfaces = fresh_rows
        .iter()
        .map(|row| row.2.as_str())
        .collect::<Vec<_>>();
    surfaces.sort_unstable();
    assert_eq!(surfaces, ["api", "managed"]);
    let mut joined_audit_ids = fresh_rows.iter().map(|row| row.0).collect::<Vec<_>>();
    joined_audit_ids.sort_unstable();
    let mut fresh_audit_ids = fresh_audit_ids;
    fresh_audit_ids.sort_unstable();
    assert_eq!(
        joined_audit_ids, fresh_audit_ids,
        "every fresh installation audit must be one of the two correlated replacement audits"
    );
    for row in &fresh_rows {
        assert_eq!(row.3, context.actor_id);
        assert_eq!(row.4, context.organization_id.as_uuid());
        assert_eq!(row.5, context.installed_uis.managed_ui.installation_id);
        assert_eq!(row.6, context.installed_uis.managed_ui.generation_id);
        assert_eq!(row.7, children_before[0].0);
        assert_eq!(row.8, UiRequestAuditDecision::Allowed.as_str());
        assert_eq!(row.9, UiRequestAuditOutcome::Succeeded.as_str());
        assert_eq!(row.10, UiRequestAuditReason::None.as_str());
        assert_eq!(row.11, "completed");
        assert_eq!(row.13, context.installed_uis.managed_gateway_id);
        assert_eq!(row.14, context.installed_uis.managed_gateway_revision_id);
        assert_eq!(row.15, Some(new_instance_id));
    }
    let fresh_invocations: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM gateway_invocations
          WHERE gateway_id = $1 AND NOT (id = ANY($2))
          ORDER BY accepted_at, id",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .bind(&baseline_invocations)
    .fetch_all(context.pool)
    .await
    .expect("read all fresh replacement gateway invocations");
    assert_eq!(fresh_invocations.len(), 2);
    let mut audit_invocations = fresh_rows.iter().map(|row| row.12).collect::<Vec<_>>();
    audit_invocations.sort_unstable();
    let mut fresh_invocations = fresh_invocations;
    fresh_invocations.sort_unstable();
    assert_eq!(
        audit_invocations, fresh_invocations,
        "replacement audits must account for exactly both fresh invocations"
    );
    assert!(new_paths.0.is_dir() && new_paths.1.is_dir() && new_paths.2.is_dir());
    write_installed_ui_control_marker(control_dir, "managed-restart-audit-verified").await;
    println!(
        "REAL_UI_INSTALLATION_LIVE_CHILD_RECOVERY=1 same_child=1 new_service_instance=1 old_resources_cleaned=1 gateway_correlation=1"
    );
}
