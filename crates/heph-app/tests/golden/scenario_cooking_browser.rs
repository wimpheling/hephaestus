use super::*;

/// Writes the initial Cooking browser fixture and runs its optional E2E/audit proof.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn run_initial_cooking_browser_phase(
    pool: &sqlx::PgPool,
    database_url: &str,
    running: &hephaestus_app::RunningHephaestus,
    _root: &Path,
    browser_e2e: bool,
    installed_reference_uis: Option<cooking_builds::InstalledCookingReferenceUis>,
    organization_id: OrganizationId,
    project_id: ProjectId,
    user_id: UserId,
    builds: &cooking_builds::PublishedCookingBuilds,
    instance: &cooking_builds::PreparedCookingInstance,
    actual_brokered: &BrokeredFixture,
    cooking_inbound_placeholder: &str,
    installed_gateway: cooking_builds::InstalledCookingGateway,
    workload_phase_timing: bool,
    mut actual_grant_id: Option<uuid::Uuid>,
) -> Option<uuid::Uuid> {
    let fixture_path = if browser_e2e {
        Some(
            env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
                .expect("cooking browser fixture output path"),
        )
    } else {
        env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT").ok()
    };
    if let Some(path) = fixture_path {
        let fixture = serde_json::json!({
            "organization_id": organization_id,
            "project_id": project_id,
            "repository_id": builds.gateway.repository_id,
            "release_id": builds.gateway.release_id,
            "release_agent_id": builds.agent.release_agent_id,
            "instance_id": instance.instance_id,
            "mailbox_id": instance.mailbox_id,
            "gateway_id": installed_gateway.gateway_id,
            "inbound_import_id": actual_brokered.import_id,
            "inbound_secret_version_id": actual_brokered.version_id,
            "inbound_selection": format!(
                "{}|{}|/cooking/telegram|x-telegram-bot-api-secret-token",
                actual_brokered.import_id, actual_brokered.version_id
            ),
            "parameters": {
                "inbound_placeholder": cooking_inbound_placeholder,
                "alice_provider_id": 1001,
                "bob_provider_id": 1002
            },
            "installed_reference_uis": installed_reference_uis.map(|uis| serde_json::json!({
                "organization_id": uis.organization_id,
                "project_id": uis.project_id,
                "repository_id": uis.repository_id,
                "static_installation_id": uis.static_ui.installation_id,
                "static_generation_id": uis.static_ui.generation_id,
                "repository_installation_id": uis.repository_static_ui.installation_id,
                "repository_generation_id": uis.repository_static_ui.generation_id,
                "global_installation_id": uis.global_static_ui.installation_id,
                "global_generation_id": uis.global_static_ui.generation_id,
                "managed_installation_id": uis.managed_ui.installation_id,
                "managed_generation_id": uis.managed_ui.generation_id
            }))
        });
        tokio::fs::write(
            &path,
            serde_json::to_vec_pretty(&fixture).expect("cooking browser fixture JSON"),
        )
        .await
        .expect("write cooking browser fixture JSON");
        if browser_e2e {
            let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
                .expect("cooking browser OIDC issuer");
            let installed_browser = installed_reference_uis.is_some();
            let script_name = if installed_browser {
                "../../scripts/run-installed-ui-e2e.sh"
            } else {
                "../../scripts/run-ui-e2e-external.sh"
            };
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(script_name);
            let browser_timer = WorkloadPhaseTimer::start("browser-initial", workload_phase_timing);
            let mut browser_command = tokio::process::Command::new(script);
            browser_command
                .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &path)
                .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", database_url)
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
                    running.http_addr().to_string(),
                )
                .env(
                    "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
                    "golden-internal-command-token-with-sufficient-entropy",
                )
                .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
                .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial");
            if installed_browser {
                browser_command
                    .env(
                        "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
                        installed_ui_platform_origin(),
                    )
                    .env("HEPHAESTUS_UI_NAMESPACE", installed_ui_namespace())
                    .env(
                        "HEPHAESTUS_CADDY_TEST_CA_CERT",
                        env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                            .expect("joined Caddy CA certificate for installed UI"),
                    );
            }
            let status = browser_command.status().await;
            browser_timer.finish(status.as_ref().is_ok_and(std::process::ExitStatus::success));
            let status = status.expect("run cooking browser E2E");
            assert!(status.success(), "cooking browser E2E failed: {status}");
            if let Some(installed_uis) = installed_reference_uis {
                let audit_actor_id = user_id.as_uuid();
                let audit_organization_id = organization_id.as_uuid();
                let static_installation_id = installed_uis.static_ui.installation_id;
                let static_generation_id = installed_uis.static_ui.generation_id;
                let managed_installation_id = installed_uis.managed_ui.installation_id;
                let managed_generation_id = installed_uis.managed_ui.generation_id;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    static_installation_id,
                    static_generation_id,
                    UiRequestAuditSurface::HandoffIssue,
                    false,
                )
                .await;
                for (installation_id, generation_id) in [
                    (
                        installed_uis.repository_static_ui.installation_id,
                        installed_uis.repository_static_ui.generation_id,
                    ),
                    (
                        installed_uis.global_static_ui.installation_id,
                        installed_uis.global_static_ui.generation_id,
                    ),
                ] {
                    for (surface, expected_success) in [
                        (UiRequestAuditSurface::HandoffIssue, false),
                        (UiRequestAuditSurface::HandoffExchange, true),
                        (UiRequestAuditSurface::Bootstrap, true),
                        (UiRequestAuditSurface::Static, true),
                    ] {
                        assert_installed_ui_audit_success(
                            pool,
                            audit_actor_id,
                            audit_organization_id,
                            installation_id,
                            generation_id,
                            surface,
                            expected_success,
                        )
                        .await;
                    }
                }
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    static_installation_id,
                    static_generation_id,
                    UiRequestAuditSurface::HandoffExchange,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    static_installation_id,
                    static_generation_id,
                    UiRequestAuditSurface::Bootstrap,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    static_installation_id,
                    static_generation_id,
                    UiRequestAuditSurface::Static,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::HandoffIssue,
                    false,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::HandoffExchange,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::Bootstrap,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::Managed,
                    true,
                )
                .await;
                let api_request_ids = assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::Api,
                    true,
                )
                .await;
                assert_installed_ui_audit_success(
                    pool,
                    audit_actor_id,
                    audit_organization_id,
                    managed_installation_id,
                    managed_generation_id,
                    UiRequestAuditSurface::Embed,
                    false,
                )
                .await;
                let correlated_api_count: i64 = sqlx::query_scalar(
                    "SELECT count(*)
                           FROM gateway_invocations invocation
                           JOIN ui_request_audit_events audit
                             ON audit.request_id = invocation.request_id
                          WHERE audit.installation_id = $1
                            AND audit.generation_id = $2
                            AND audit.surface = $3
                            AND audit.request_id = ANY($4)",
                )
                .bind(managed_installation_id)
                .bind(managed_generation_id)
                .bind(UiRequestAuditSurface::Api.as_str())
                .bind(&api_request_ids)
                .fetch_one(pool)
                .await
                .expect("read managed UI gateway invocation correlation");
                assert!(
                    correlated_api_count > 0,
                    "managed API audit must correlate to a gateway invocation"
                );
                println!(
                    "REAL_UI_INSTALLATION_AUDIT=1 static=1 managed=1 api=1 embed=1 gateway_correlation=1"
                );
            }
            let browser_gateway_id = installed_gateway.gateway_id;
            let active_revision_id: uuid::Uuid =
                sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
                    .bind(browser_gateway_id)
                    .fetch_one(pool)
                    .await
                    .expect("cooking browser active gateway revision");
            assert_ne!(
                active_revision_id, installed_gateway.revision_id,
                "browser must create a new gateway revision"
            );
            actual_grant_id = Some(
                sqlx::query_scalar(
                    "SELECT binding_grant.id
                     FROM gateways gateway
                     JOIN gateway_revisions revision
                       ON revision.gateway_id = gateway.id
                      AND revision.id = gateway.active_revision_id
                     JOIN gateway_mailbox_bindings binding
                       ON binding.gateway_revision_id = revision.id
                      AND binding.mailbox_id = $1
                     JOIN gateway_mailbox_binding_grants binding_grant
                       ON binding_grant.binding_id = binding.id
                      AND binding_grant.status = 'active'
                     WHERE gateway.id = $2
                     ORDER BY binding_grant.granted_at DESC, binding_grant.id DESC
                         LIMIT 1",
                )
                .bind(instance.mailbox_id)
                .bind(browser_gateway_id)
                .fetch_one(pool)
                .await
                .expect("cooking browser active binding grant"),
            );
        }
    }
    actual_grant_id
}
