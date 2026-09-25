use super::*;

pub const ISSUER: &str = "https://issuer.golden.invalid";

/// Returns the issuer used by this golden run, including the isolated issuer
/// supplied when the cooking browser attaches to the live daemon.
pub fn golden_issuer() -> String {
    env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER").unwrap_or_else(|_| ISSUER.to_owned())
}
pub const AUDIENCE: &str = "hephaestus-git";
pub const SIGNING_SECRET: &[u8] = b"golden-test-signing-secret-with-sufficient-entropy";
pub const ROOT_IMAGE: &str =
    "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const BROKERED_E2E_RULE_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
pub const BROKERED_E2E_SENTINEL: &str = "golden-brokered-provider-sentinel-5d1a";
pub const GATEWAY_HANDLER: &str =
    "#!/bin/sh\nexec /usr/libexec/hephaestus/integration-check --private-http-brokered-mailbox\n";
pub const SERVICE_GATEWAY_HANDLER: &str =
    "#!/bin/sh\nexec /usr/libexec/hephaestus/integration-check --serve-service\n";

pub async fn restart_application(config: AppConfig) -> hephaestus_app::RunningHephaestus {
    HephaestusApp::build(config)
        .await
        .expect("rebuild production application after restart")
        .start()
        .await
        .expect("restart ready application")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductOutboxCensus {
    pub total: i64,
    pub pending: i64,
    pub published: i64,
    pub dead_lettered: i64,
    pub pending_fingerprint: String,
}

pub struct IsolatedGoldenDatabase {
    pub database_name: String,
    pub maintenance_url: String,
    pub target_url: String,
}

/// Verifies only the redacted, safe context emitted by the installed UI
/// browser flow. The request audit stream deliberately has no route, query,
/// cookie, credential, or response-payload columns for this observer to read.
pub async fn assert_installed_ui_audit_success(
    pool: &sqlx::PgPool,
    actor_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    installation_id: uuid::Uuid,
    generation_id: uuid::Uuid,
    surface: UiRequestAuditSurface,
    child_required: bool,
) -> Vec<uuid::Uuid> {
    type AuditRow = (
        uuid::Uuid,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
        Option<uuid::Uuid>,
    );
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT request_id, actor_id, organization_id, installation_id,
                generation_id, child_session_id, gateway_id, gateway_revision_id
           FROM ui_request_audit_events
          WHERE installation_id = $1
            AND generation_id = $2
            AND surface = $3
            AND decision = $4
            AND outcome = $5
            AND reason_code = $6
          ORDER BY occurred_at, id",
    )
    .bind(installation_id)
    .bind(generation_id)
    .bind(surface.as_str())
    .bind(UiRequestAuditDecision::Allowed.as_str())
    .bind(UiRequestAuditOutcome::Succeeded.as_str())
    .bind(UiRequestAuditReason::None.as_str())
    .fetch_all(pool)
    .await
    .expect("read installed UI request audit rows");
    assert!(
        !rows.is_empty(),
        "installed UI must emit a successful {} audit row",
        surface.as_str()
    );
    for row in &rows {
        assert_eq!(row.1, Some(actor_id), "{} audit actor", surface.as_str());
        assert_eq!(
            row.2,
            Some(organization_id),
            "{} audit organization",
            surface.as_str()
        );
        assert_eq!(
            row.3,
            Some(installation_id),
            "{} audit installation",
            surface.as_str()
        );
        assert_eq!(
            row.4,
            Some(generation_id),
            "{} audit generation",
            surface.as_str()
        );
        assert_eq!(
            row.5.is_some(),
            child_required,
            "{} audit child-session context",
            surface.as_str()
        );
        assert_eq!(
            row.6.is_some(),
            row.7.is_some(),
            "{} audit gateway context must be paired",
            surface.as_str()
        );
    }
    rows.into_iter().map(|row| row.0).collect()
}

pub struct InstalledUiBrowserContext<'a> {
    pub pool: &'a sqlx::PgPool,
    pub running: &'a hephaestus_app::RunningHephaestus,
    pub database_url: &'a str,
    pub rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    pub organization_id: OrganizationId,
    pub installed_uis: cooking_builds::InstalledCookingReferenceUis,
    pub actor_id: uuid::Uuid,
    pub workload_phase_timing: bool,
    pub service_materializer_root: &'a Path,
}

pub const INSTALLED_UI_CONTROL_MARKERS: [&str; 25] = [
    "managed-ready",
    "guest-policy-ready",
    "guest-policy-start",
    "guest-policy-complete",
    "guest-policy-verified",
    "disable-complete",
    "stale-cookie-denied",
    "reactivate-complete",
    "old-generation-denied-after-reactivate",
    "old-generation-denial-verified",
    "new-generation-ready",
    "new-generation-verified",
    "parent-revoke-ready",
    "parent-revoke-permitted",
    "parent-revoked-denied",
    "parent-revocation-verified",
    "parent-new-child-ready",
    "remove-ready",
    "remove-complete",
    "removed-host-denied",
    "removed-card-absent",
    "managed-restart-ready",
    "managed-restart-complete",
    "managed-restart-verified",
    "managed-restart-audit-verified",
];
pub const INSTALLED_UI_LIFECYCLE_DEADLINE: Duration = Duration::from_secs(360);
pub type InstalledUiDenialRow = (
    String,
    String,
    String,
    String,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
);
pub type GuestPolicyAuditRow = (
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    String,
    String,
    String,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
    uuid::Uuid,
    uuid::Uuid,
    uuid::Uuid,
    String,
);
