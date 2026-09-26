use super::*;

pub struct InstalledUiParentRevocationBaseline {
    pub child_id: uuid::Uuid,
    pub parent_id: uuid::Uuid,
    pub content_audit_ids: Vec<uuid::Uuid>,
    pub managed_audit_ids: Vec<uuid::Uuid>,
    pub gateway_invocations: i64,
    pub boundary: OffsetDateTime,
}

pub async fn snapshot_installed_ui_parent_revocation(
    context: &InstalledUiBrowserContext<'_>,
    activated_context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
    deadline: tokio::time::Instant,
) -> InstalledUiParentRevocationBaseline {
    wait_for_installed_ui_control_marker(control_dir, "parent-revoke-ready", deadline).await;
    let existing_children = installed_ui_current_generation_children(activated_context).await;
    assert_eq!(
        existing_children.len(),
        1,
        "current generation must have exactly one child before parent logout"
    );
    let (child_id, parent_id) = existing_children[0];
    let parent_before: (uuid::Uuid, Option<OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT user_id, revoked_at, revocation_reason
           FROM human_browser_sessions
          WHERE id = $1",
    )
    .bind(parent_id)
    .fetch_one(context.pool)
    .await
    .expect("read current installed UI parent session before logout");
    assert_eq!(
        parent_before.0, context.actor_id,
        "current UI parent must belong to the golden actor"
    );
    assert!(
        parent_before.1.is_none() && parent_before.2.is_none(),
        "current UI parent must be active before logout"
    );
    let content_audit_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id
           FROM ui_request_audit_events
          WHERE surface = 'content'
          ORDER BY occurred_at, id",
    )
    .fetch_all(context.pool)
    .await
    .expect("snapshot content audit IDs before parent logout");
    let managed_audit_ids = installed_ui_audit_ids(activated_context).await;
    let gateway_invocations = installed_ui_gateway_invocations(activated_context).await;
    let boundary: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(context.pool)
        .await
        .expect("read parent logout audit boundary");

    InstalledUiParentRevocationBaseline {
        child_id,
        parent_id,
        content_audit_ids,
        managed_audit_ids,
        gateway_invocations,
        boundary,
    }
}

pub async fn assert_installed_ui_parent_revocation(
    context: &InstalledUiBrowserContext<'_>,
    activated_context: &InstalledUiBrowserContext<'_>,
    baseline: &InstalledUiParentRevocationBaseline,
) {
    let parent_after: (uuid::Uuid, Option<OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT parent.user_id, parent.revoked_at, parent.revocation_reason
           FROM ui_browser_sessions AS child
           JOIN human_browser_sessions AS parent
             ON parent.id = child.parent_session_id
          WHERE child.id = $1
            AND child.parent_session_id = $2
            AND child.installation_id = $3
            AND child.generation_id = $4",
    )
    .bind(baseline.child_id)
    .bind(baseline.parent_id)
    .bind(activated_context.installed_uis.managed_ui.installation_id)
    .bind(activated_context.installed_uis.managed_ui.generation_id)
    .fetch_one(context.pool)
    .await
    .expect("verify revoked parent through child-parent join");
    assert_eq!(
        parent_after.0, context.actor_id,
        "revoked UI parent must retain its actor"
    );
    assert!(
        parent_after.1.is_some(),
        "logout must revoke the authoritative parent session"
    );
    assert_eq!(
        parent_after.2.as_deref(),
        Some("logout"),
        "logout must persist the logout revocation reason"
    );
    assert_installed_ui_stale_cookie_denial(
        activated_context,
        baseline.boundary,
        &baseline.content_audit_ids,
        UiRequestAuditReason::Unauthenticated,
    )
    .await;
    assert_eq!(
        installed_ui_audit_ids(activated_context).await,
        baseline.managed_audit_ids,
        "revoked child must not create a managed-context audit"
    );
    assert_eq!(
        installed_ui_gateway_invocations(activated_context).await,
        baseline.gateway_invocations,
        "revoked child must not invoke the managed gateway"
    );
}

pub async fn assert_installed_ui_relogin_child(
    context: &InstalledUiBrowserContext<'_>,
    activated_context: &InstalledUiBrowserContext<'_>,
    baseline: &InstalledUiParentRevocationBaseline,
    existing_children: &[(uuid::Uuid, uuid::Uuid)],
    control_dir: &Path,
    deadline: tokio::time::Instant,
) {
    wait_for_installed_ui_control_marker(control_dir, "parent-new-child-ready", deadline).await;
    let new_children = installed_ui_current_generation_children(activated_context).await;
    let fresh_children: Vec<(uuid::Uuid, uuid::Uuid)> = new_children
        .iter()
        .copied()
        .filter(|(child_id, _)| {
            !existing_children
                .iter()
                .any(|(old_id, _)| old_id == child_id)
        })
        .collect();
    assert_eq!(
        fresh_children.len(),
        1,
        "re-login must create exactly one fresh current-generation child"
    );
    let (new_child_id, new_parent_id) = fresh_children[0];
    assert_ne!(
        new_parent_id, baseline.parent_id,
        "re-login must use a new browser parent session"
    );
    let new_parent: (uuid::Uuid, Option<OffsetDateTime>, Option<String>) = sqlx::query_as(
        "SELECT user_id, revoked_at, revocation_reason
           FROM human_browser_sessions
          WHERE id = $1",
    )
    .bind(new_parent_id)
    .fetch_one(context.pool)
    .await
    .expect("read re-login parent session");
    assert_eq!(new_parent.0, context.actor_id);
    assert!(
        new_parent.1.is_none() && new_parent.2.is_none(),
        "re-login parent must be active"
    );
    // Browser content audits intentionally omit gateway references; the
    // request-ID join and invocation binding below provide the gateway proof.
    let new_child_surfaces: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT audit.surface
           FROM ui_request_audit_events AS audit
           JOIN gateway_invocations AS invocation
             ON invocation.request_id = audit.request_id
          WHERE audit.actor_id = $1
            AND audit.organization_id = $2
            AND audit.installation_id = $3
            AND audit.generation_id = $4
            AND audit.child_session_id = $5
            AND audit.occurred_at >= $8
            AND audit.decision = 'allowed'
            AND audit.outcome = 'succeeded'
            AND audit.reason_code = 'none'
            AND audit.surface IN ('managed', 'api')
            AND invocation.gateway_id = $6
            AND invocation.gateway_revision_id = $7
            AND invocation.outcome = 'completed'
          ORDER BY audit.surface",
    )
    .bind(context.actor_id)
    .bind(activated_context.organization_id.as_uuid())
    .bind(activated_context.installed_uis.managed_ui.installation_id)
    .bind(activated_context.installed_uis.managed_ui.generation_id)
    .bind(new_child_id)
    .bind(activated_context.installed_uis.managed_gateway_id)
    .bind(activated_context.installed_uis.managed_gateway_revision_id)
    .bind(baseline.boundary)
    .fetch_all(context.pool)
    .await
    .expect("read re-login audits correlated to completed gateway invocations");
    assert!(
        new_child_surfaces
            .iter()
            .any(|surface| surface == "managed"),
        "re-login child must create a managed success audit"
    );
    assert!(
        new_child_surfaces.iter().any(|surface| surface == "api"),
        "re-login child must create an API success audit"
    );
    assert!(
        installed_ui_gateway_invocations(activated_context).await > baseline.gateway_invocations,
        "re-login child must create a fresh managed gateway invocation"
    );
    write_installed_ui_control_marker(control_dir, "new-generation-verified").await;
}
