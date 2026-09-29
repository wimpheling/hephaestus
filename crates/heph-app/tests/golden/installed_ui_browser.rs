use super::*;

pub async fn run_installed_ui_disable_control(
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
) {
    let deadline = tokio::time::Instant::now() + INSTALLED_UI_LIFECYCLE_DEADLINE;
    run_installed_ui_guest_policy_phase(context, control_dir, deadline).await;
    let (baseline_managed_audit_ids, baseline_managed_invocations, reactivation_content_audit_ids) =
        run_installed_ui_disable_phase(context, control_dir, deadline).await;
    let reactivated = run_installed_ui_reactivation_phase(
        context,
        control_dir,
        deadline,
        &baseline_managed_audit_ids,
        baseline_managed_invocations,
        &reactivation_content_audit_ids,
    )
    .await;
    let mut activated_uis = context.installed_uis;
    activated_uis.managed_ui = reactivated;
    let activated_context = InstalledUiBrowserContext {
        pool: context.pool,
        running: context.running,
        database_url: context.database_url,
        rpc_token: context.rpc_token,
        organization_id: context.organization_id,
        installed_uis: activated_uis,
        actor_id: context.actor_id,
        workload_phase_timing: context.workload_phase_timing,
        service_materializer_root: context.service_materializer_root,
    };
    assert_installed_ui_browser_audit(&activated_context).await;
    let new_generation_invocations: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM gateway_invocations
          WHERE gateway_id = $1",
    )
    .bind(context.installed_uis.managed_gateway_id)
    .fetch_one(context.pool)
    .await
    .expect("count managed gateway invocations after reactivation");
    assert!(
        new_generation_invocations > baseline_managed_invocations,
        "new managed generation must create a gateway invocation"
    );
    run_installed_ui_live_service_replacement(&activated_context, control_dir, deadline).await;
    run_installed_ui_parent_revocation_phase(context, &activated_context, control_dir, deadline)
        .await;
    run_installed_ui_removal_phase(context, &activated_context, control_dir, deadline).await;
    println!(
        "REAL_UI_INSTALLATION_DISABLE_LIFECYCLE=1 stale_cookie_denied=1 old_generation_denied=1 gateway_invocation_unchanged=1 reactivated_generation=1 new_generation_audit=1 gateway_invocation_increased=1"
    );
}

/// Emits only bounded, closed-vocabulary evidence when the browser process
/// fails. The browser reporter intentionally exposes only an HTTP status
/// class, so this joins the managed installation's audit rows to durable
/// gateway outcomes without reading request or response data.
pub async fn diagnose_installed_ui_browser_failure(context: &InstalledUiBrowserContext<'_>) {
    type AuditGroup = (String, String, String, String, i64);
    type InvocationGroup = (String, String, i64);
    let installation_id = context.installed_uis.managed_ui.installation_id;
    let generation_id = context.installed_uis.managed_ui.generation_id;
    let query = async {
        let audits: Vec<AuditGroup> = sqlx::query_as(
            "SELECT surface, decision, outcome, reason_code, count(*)
               FROM ui_request_audit_events
              WHERE installation_id = $1 AND generation_id = $2
              GROUP BY surface, decision, outcome, reason_code
              ORDER BY surface, decision, outcome, reason_code",
        )
        .bind(installation_id)
        .bind(generation_id)
        .fetch_all(context.pool)
        .await?;
        let invocations: Vec<InvocationGroup> = sqlx::query_as(
            "SELECT audit.surface, invocation.outcome, count(DISTINCT invocation.id)
               FROM ui_request_audit_events AS audit
               JOIN gateway_invocations AS invocation
                 ON invocation.request_id = audit.request_id
              WHERE audit.installation_id = $1 AND audit.generation_id = $2
              GROUP BY audit.surface, invocation.outcome
              ORDER BY audit.surface, invocation.outcome",
        )
        .bind(installation_id)
        .bind(generation_id)
        .fetch_all(context.pool)
        .await?;
        Ok::<_, sqlx::Error>((audits, invocations))
    };
    match tokio::time::timeout(Duration::from_secs(5), query).await {
        Ok(Ok((audits, invocations))) => {
            println!(
                "INSTALLED_UI_BROWSER_FAILURE_DIAGNOSTIC=1 audit_groups={} invocation_groups={}",
                audits.len(),
                invocations.len()
            );
            for (surface, decision, outcome, reason, count) in audits {
                println!(
                    "INSTALLED_UI_AUDIT_GROUP surface={surface} decision={decision} outcome={outcome} reason={reason} count={count}"
                );
            }
            for (surface, outcome, count) in invocations {
                println!(
                    "INSTALLED_UI_GATEWAY_OUTCOME surface={surface} outcome={outcome} count={count}"
                );
            }
        }
        Ok(Err(_)) => println!("INSTALLED_UI_BROWSER_FAILURE_DIAGNOSTIC=1 status=unavailable"),
        Err(_) => println!("INSTALLED_UI_BROWSER_FAILURE_DIAGNOSTIC=1 status=timeout"),
    }
}

/// Runs the installed-reference-UI browser phase while the service-proof
/// daemon, Caddy, database, and managed gateway are still alive.  The
/// service-proof branch otherwise tears those resources down before reaching
/// the ordinary browser block below.
pub async fn supervise_installed_ui_browser(
    mut browser: tokio::process::Child,
    context: &InstalledUiBrowserContext<'_>,
    control_dir: &Path,
) -> std::process::ExitStatus {
    use futures_util::FutureExt as _;

    let mut disable_control = Box::pin(
        std::panic::AssertUnwindSafe(tokio::time::timeout(
            INSTALLED_UI_LIFECYCLE_DEADLINE,
            run_installed_ui_disable_control(context, control_dir),
        ))
        .catch_unwind(),
    );
    tokio::select! {
        result = browser.wait() => {
            let status = result.expect("wait for installed UI browser E2E");
            if status.success() {
                match (&mut disable_control).await {
                    Err(panic) => std::panic::resume_unwind(panic),
                    Ok(Err(_)) => panic!("installed UI disable controller deadline elapsed"),
                    Ok(Ok(())) => {}
                }
            } else {
                println!("INSTALLED_UI_DISABLE_CONTROLLER_CANCELLED=1 browser_status=failed");
                drop(disable_control);
            }
            status
        },
        control_result = &mut disable_control => {
            match control_result {
                Err(panic) => {
                    let _ = browser.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(5), browser.wait()).await;
                    std::panic::resume_unwind(panic);
                }
                Ok(Err(_)) => {
                    let _ = browser.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(5), browser.wait()).await;
                    panic!("installed UI disable controller deadline elapsed");
                }
                Ok(Ok(())) => {}
            }
            tokio::time::timeout(INSTALLED_UI_LIFECYCLE_DEADLINE, browser.wait())
                .await
                .expect("installed UI browser E2E completion deadline")
                .expect("wait for installed UI browser E2E after disable")
        }
    }
}

pub async fn run_installed_ui_browser_phase(context: InstalledUiBrowserContext<'_>) {
    let fixture_path = env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
        .expect("installed UI browser fixture output path");
    let fixture = serde_json::json!({
        "installed_reference_uis": {
            "organization_id": context.installed_uis.organization_id,
            "project_id": context.installed_uis.project_id,
            "repository_id": context.installed_uis.repository_id,
            "static_installation_id": context.installed_uis.static_ui.installation_id,
            "repository_installation_id": context.installed_uis.repository_static_ui.installation_id,
            "global_installation_id": context.installed_uis.global_static_ui.installation_id,
            "managed_installation_id": context.installed_uis.managed_ui.installation_id
        }
    });
    tokio::fs::write(
        &fixture_path,
        serde_json::to_vec_pretty(&fixture).expect("installed UI browser fixture JSON"),
    )
    .await
    .expect("write installed UI browser fixture JSON");
    let issuer = env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
        .expect("installed UI browser OIDC issuer");
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-installed-ui-e2e.sh");
    let browser_timer = WorkloadPhaseTimer::start("browser-initial", context.workload_phase_timing);
    let control_dir = prepare_installed_ui_control_dir(Path::new(&fixture_path));
    let browser = tokio::process::Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &fixture_path)
        .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", context.database_url)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            context.running.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial")
        .env(
            "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
            installed_ui_platform_origin(),
        )
        .env("HEPHAESTUS_UI_NAMESPACE", installed_ui_namespace())
        .env(
            "HEPHAESTUS_CADDY_TEST_CA_CERT",
            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                .expect("joined Caddy CA certificate for installed UI"),
        )
        .kill_on_drop(true)
        .spawn()
        .expect("spawn installed UI browser E2E");
    let status = supervise_installed_ui_browser(browser, &context, &control_dir).await;
    browser_timer.finish(status.success());
    if !status.success() {
        diagnose_installed_ui_browser_failure(&context).await;
    }
    assert!(
        status.success(),
        "installed UI browser E2E failed: {status}"
    );

    assert_installed_ui_browser_audit(&context).await;
}

// Keep the positive audit matrix together so each installed owner surface is
// checked before lifecycle control proceeds.
#[allow(clippy::too_many_lines)]
pub async fn assert_installed_ui_browser_audit(context: &InstalledUiBrowserContext<'_>) {
    let static_installation_id = context.installed_uis.static_ui.installation_id;
    let static_generation_id = context.installed_uis.static_ui.generation_id;
    for surface in [
        UiRequestAuditSurface::HandoffIssue,
        UiRequestAuditSurface::HandoffExchange,
        UiRequestAuditSurface::Bootstrap,
        UiRequestAuditSurface::Static,
    ] {
        assert_installed_ui_audit_success(
            context.pool,
            context.actor_id,
            context.organization_id.as_uuid(),
            static_installation_id,
            static_generation_id,
            surface,
            surface != UiRequestAuditSurface::HandoffIssue,
        )
        .await;
    }
    for (installation_id, generation_id) in [
        (
            context.installed_uis.repository_static_ui.installation_id,
            context.installed_uis.repository_static_ui.generation_id,
        ),
        (
            context.installed_uis.global_static_ui.installation_id,
            context.installed_uis.global_static_ui.generation_id,
        ),
    ] {
        for surface in [
            UiRequestAuditSurface::HandoffIssue,
            UiRequestAuditSurface::HandoffExchange,
            UiRequestAuditSurface::Bootstrap,
            UiRequestAuditSurface::Static,
        ] {
            assert_installed_ui_audit_success(
                context.pool,
                context.actor_id,
                context.organization_id.as_uuid(),
                installation_id,
                generation_id,
                surface,
                surface != UiRequestAuditSurface::HandoffIssue,
            )
            .await;
        }
    }
    let managed_installation_id = context.installed_uis.managed_ui.installation_id;
    let managed_generation_id = context.installed_uis.managed_ui.generation_id;
    for surface in [
        UiRequestAuditSurface::HandoffIssue,
        UiRequestAuditSurface::HandoffExchange,
        UiRequestAuditSurface::Bootstrap,
        UiRequestAuditSurface::Managed,
        UiRequestAuditSurface::Embed,
    ] {
        assert_installed_ui_audit_success(
            context.pool,
            context.actor_id,
            context.organization_id.as_uuid(),
            managed_installation_id,
            managed_generation_id,
            surface,
            !matches!(
                surface,
                UiRequestAuditSurface::HandoffIssue | UiRequestAuditSurface::Embed
            ),
        )
        .await;
    }
    let api_request_ids = assert_installed_ui_audit_success(
        context.pool,
        context.actor_id,
        context.organization_id.as_uuid(),
        managed_installation_id,
        managed_generation_id,
        UiRequestAuditSurface::Api,
        true,
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
    .fetch_one(context.pool)
    .await
    .expect("read installed UI gateway invocation correlation");
    assert!(
        correlated_api_count > 0,
        "managed API audit must correlate to a gateway invocation"
    );
    println!("REAL_UI_INSTALLATION_AUDIT=1 static=1 managed=1 api=1 embed=1 gateway_correlation=1");
}
