use super::*;

pub async fn snapshot_guest_policy_boundary(
    context: &InstalledUiBrowserContext<'_>,
) -> (Vec<uuid::Uuid>, Vec<uuid::Uuid>, OffsetDateTime) {
    let installation_id = context.installed_uis.managed_ui.installation_id;
    let generation_id = context.installed_uis.managed_ui.generation_id;
    let gateway_id = context.installed_uis.managed_gateway_id;
    let api_audits = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1 AND generation_id = $2 AND surface = 'api'
          ORDER BY occurred_at, id",
    )
    .bind(installation_id)
    .bind(generation_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot installed UI API audits before guest policy probes");
    let invocations = sqlx::query_scalar(
        "SELECT id
           FROM gateway_invocations
          WHERE gateway_id = $1
          ORDER BY accepted_at, id",
    )
    .bind(gateway_id)
    .fetch_all(context.pool)
    .await
    .expect("snapshot direct installed UI gateway invocations");
    let boundary = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(context.pool)
        .await
        .expect("read guest policy audit boundary");
    (api_audits, invocations, boundary)
}

pub async fn read_guest_policy_audits(
    context: &InstalledUiBrowserContext<'_>,
    boundary: OffsetDateTime,
    baseline_api_audit_ids: &[uuid::Uuid],
) -> Vec<GuestPolicyAuditRow> {
    sqlx::query_as(
        "SELECT audit.id, audit.request_id, audit.actor_id,
                audit.organization_id, audit.installation_id,
                audit.generation_id, audit.child_session_id,
                audit.decision, audit.outcome, audit.reason_code,
                audit.gateway_id, audit.gateway_revision_id,
                invocation.id, invocation.gateway_id,
                invocation.gateway_revision_id, invocation.outcome
           FROM ui_request_audit_events AS audit
           JOIN gateway_invocations AS invocation
             ON invocation.request_id = audit.request_id
          WHERE audit.installation_id = $1
            AND audit.generation_id = $2
            AND audit.surface = 'api'
            AND audit.occurred_at >= $3
            AND NOT (audit.id = ANY($4))
          ORDER BY audit.occurred_at, audit.id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .bind(boundary)
    .bind(baseline_api_audit_ids)
    .fetch_all(context.pool)
    .await
    .expect("read guest policy API audits correlated to gateway invocations")
}

pub async fn read_guest_policy_audit_ids(
    context: &InstalledUiBrowserContext<'_>,
    boundary: OffsetDateTime,
    baseline_api_audit_ids: &[uuid::Uuid],
) -> Vec<uuid::Uuid> {
    sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE installation_id = $1
            AND generation_id = $2
            AND surface = 'api'
            AND occurred_at >= $3
            AND NOT (id = ANY($4))
          ORDER BY occurred_at, id",
    )
    .bind(context.installed_uis.managed_ui.installation_id)
    .bind(context.installed_uis.managed_ui.generation_id)
    .bind(boundary)
    .bind(baseline_api_audit_ids)
    .fetch_all(context.pool)
    .await
    .expect("read all fresh guest policy API audit IDs")
}

pub fn assert_guest_policy_audits(
    context: &InstalledUiBrowserContext<'_>,
    rows: &[GuestPolicyAuditRow],
    fresh_invocation_ids: &[uuid::Uuid],
) {
    assert_eq!(
        rows.len(),
        4,
        "guest policy probes must emit four API audits"
    );
    let mut request_ids: Vec<uuid::Uuid> = rows.iter().map(|row| row.1).collect();
    request_ids.sort_unstable();
    request_ids.dedup();
    assert_eq!(
        request_ids.len(),
        4,
        "guest policy probes need distinct request IDs"
    );
    let mut completed = 0;
    let mut failed = 0;
    let mut failed_audit_count = 0;
    let mut succeeded_audit_count = 0;
    let mut child_session_id = None;
    for row in rows {
        assert_eq!(row.2, context.actor_id, "guest policy audit actor");
        assert_eq!(row.3, context.organization_id.as_uuid());
        assert_eq!(row.4, context.installed_uis.managed_ui.installation_id);
        assert_eq!(row.5, context.installed_uis.managed_ui.generation_id);
        if let Some(existing) = child_session_id {
            assert_eq!(row.6, existing, "guest policy probes must use one child");
        } else {
            child_session_id = Some(row.6);
        }
        assert_eq!(row.7, UiRequestAuditDecision::Allowed.as_str());
        assert!(row.10.is_none() && row.11.is_none());
        assert!(fresh_invocation_ids.contains(&row.12));
        assert_eq!(row.13, context.installed_uis.managed_gateway_id);
        assert_eq!(row.14, context.installed_uis.managed_gateway_revision_id);
        match row.8.as_str() {
            "succeeded" => {
                succeeded_audit_count += 1;
                assert_eq!(row.9, UiRequestAuditReason::None.as_str());
                assert_eq!(row.15, "completed");
                completed += 1;
            }
            "failed" => {
                failed_audit_count += 1;
                assert_eq!(row.9, UiRequestAuditReason::UpstreamFailure.as_str());
                match row.15.as_str() {
                    "completed" => completed += 1,
                    "failed" => failed += 1,
                    outcome => panic!("unexpected guest policy invocation outcome {outcome}"),
                }
            }
            outcome => panic!("unexpected guest policy audit outcome {outcome}"),
        }
    }
    assert_eq!(succeeded_audit_count, 1);
    assert_eq!(failed_audit_count, 3);
    assert_eq!(failed, 1);
    assert_eq!(completed, 3);
}

/// Verifies the installed guest's four response-policy probes before the
/// lifecycle controller starts mutating the installation.
pub async fn run_installed_ui_guest_policy_phase(
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) {
    wait_for_installed_ui_control_marker(control_dir, "guest-policy-ready", deadline).await;
    let (baseline_api_audit_ids, baseline_invocation_ids, boundary) =
        snapshot_guest_policy_boundary(context).await;
    write_installed_ui_control_marker(control_dir, "guest-policy-start").await;
    wait_for_installed_ui_control_marker(control_dir, "guest-policy-complete", deadline).await;
    let invocation_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM gateway_invocations
          WHERE gateway_id = $1
          ORDER BY accepted_at, id",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_all(context.pool)
    .await
    .expect("read direct installed UI gateway invocations after guest policy probes");
    let fresh_invocation_ids: Vec<uuid::Uuid> = invocation_ids
        .iter()
        .copied()
        .filter(|id| !baseline_invocation_ids.contains(id))
        .collect();
    assert_eq!(fresh_invocation_ids.len(), 4);
    let fresh_audit_ids =
        read_guest_policy_audit_ids(context, boundary, &baseline_api_audit_ids).await;
    assert_eq!(fresh_audit_ids.len(), 4);
    let rows = read_guest_policy_audits(context, boundary, &baseline_api_audit_ids).await;
    let mut joined_audit_ids: Vec<uuid::Uuid> = rows.iter().map(|row| row.0).collect();
    let mut fresh_audit_ids = fresh_audit_ids;
    joined_audit_ids.sort_unstable();
    fresh_audit_ids.sort_unstable();
    assert_eq!(
        joined_audit_ids, fresh_audit_ids,
        "every fresh guest policy API audit must correlate to one gateway invocation"
    );
    assert_guest_policy_audits(context, &rows, &fresh_invocation_ids);
    write_installed_ui_control_marker(control_dir, "guest-policy-verified").await;
    println!(
        "REAL_UI_INSTALLATION_GUEST_POLICY=1 credential_headers_stripped=1 guest_response_headers_rejected=3 gateway_correlation=1"
    );
}
