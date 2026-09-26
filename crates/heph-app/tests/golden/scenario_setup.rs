use super::*;

/// Validated feature gates and disposable database state for the production golden run.
// Some opt-in gates are retained in the mode record for fixture compatibility
// even when a particular golden path does not consume them directly.
#[allow(clippy::struct_excessive_bools, dead_code)]
pub struct GoldenRunMode {
    pub workload_phase_timing: bool,
    pub parent_database_url: String,
    pub nats_url: String,
    pub parent_before: Option<ProductOutboxCensus>,
    pub isolated_database: IsolatedGoldenDatabase,
    pub database_url: String,
    pub libkrun_e2e: bool,
    pub session_chat_e2e: bool,
    pub session_chat_denial_probe_e2e: bool,
    pub session_chat_restart_e2e: bool,
    pub session_chat_browser_e2e: bool,
    pub installed_ui_fixture: bool,
    pub cooking_build_proof: bool,
    pub release_build_proof: bool,
    pub cooking_service_build_proof: bool,
    pub caddy_tls: bool,
    pub build_timeout: Duration,
    pub cooking_wait_timeout: Duration,
    pub browser_e2e: bool,
    pub gateway_caddy_e2e: bool,
    pub gateway_service_e2e: bool,
    pub gateway_service_external_e2e: bool,
    pub gateway_service_revocation_e2e: bool,
    pub gateway_service_cutover_e2e: bool,
    pub gateway_service_failed_candidate_e2e: bool,
    pub gateway_service_candidate_capacity_e2e: bool,
    pub gateway_service_rollback_e2e: bool,
    pub gateway_service_log_rpc_e2e: bool,
    pub gateway_service_log_guest_e2e: bool,
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare_golden_run_mode() -> Option<GoldenRunMode> {
    drop(
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
                    .add_directive(
                        "gateway_postgres::ui_post_admission=warn"
                            .parse()
                            .expect("valid installed UI diagnostic filter"),
                    ),
            )
            .with_test_writer()
            .try_init(),
    );
    let workload_phase_timing = workload_phase_timing_from_environment();
    let (Ok(parent_database_url), Ok(nats_url)) = (
        std::env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        std::env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        assert!(
            !cooking::enabled()
                && env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() != Ok("1")
                && env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() != Ok("1"),
            "explicit E2E execution requires HEPHAESTUS_POSTGRES_TEST_URL and HEPHAESTUS_NATS_TEST_URL"
        );
        return None;
    };
    // The parent census is an explicit standalone verification mode. Ordinary
    // golden runs may share the parent database with legitimate writers, so
    // they never assert a whole-parent invariant.
    let parent_before =
        if env::var("HEPHAESTUS_GOLDEN_ASSERT_PARENT_ISOLATION").as_deref() == Ok("1") {
            Some(product_outbox_census(&parent_database_url).await)
        } else {
            None
        };
    let isolated_database = IsolatedGoldenDatabase::create(&parent_database_url).await;
    let database_url = isolated_database.target_url.clone();
    let libkrun_e2e = env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1");
    let session_chat_e2e = session_chat::enabled();
    let session_chat_denial_probe_e2e = session_chat::denial_probe_enabled();
    let session_chat_restart_e2e = session_chat::restart_e2e_enabled();
    let session_chat_browser_e2e =
        env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1");
    assert!(
        !session_chat_browser_e2e || session_chat_e2e,
        "session-chat browser E2E requires the standalone session-chat scenario"
    );
    assert!(
        !session_chat_denial_probe_e2e || session_chat_e2e,
        "session-chat denial probe requires the standalone session-chat scenario"
    );
    assert!(
        !session_chat_denial_probe_e2e || !session_chat_browser_e2e,
        "session-chat denial probe is a standalone session mode"
    );
    assert!(
        !session_chat_restart_e2e || session_chat_browser_e2e,
        "session-chat restart acceptance requires the browser phase"
    );
    let installed_ui_fixture = env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref()
        == Ok("1")
        || session_chat_browser_e2e;
    let cooking_build_proof = env::var("HEPHAESTUS_APP_COOKING_BUILD_PROOF").as_deref() == Ok("1");
    let release_build_proof = cooking_build_proof || session_chat_e2e;
    let cooking_service_build_proof =
        env::var("HEPHAESTUS_APP_COOKING_SERVICE_BUILD_PROOF").as_deref() == Ok("1");
    let caddy_tls = env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1");
    // Ordinary golden tests keep their short timeout. The real Cooking proof
    // uses the production build limit, with a small margin for the observer's
    // final state poll and cleanup.
    let build_timeout = if release_build_proof {
        Duration::from_secs(15 * 60)
    } else {
        Duration::from_secs(30)
    };
    let cooking_wait_timeout = if release_build_proof {
        build_timeout + Duration::from_secs(30)
    } else {
        Duration::from_secs(300)
    };
    let browser_e2e = env::var("HEPHAESTUS_COOKING_BROWSER_E2E").as_deref() == Ok("1");
    assert!(
        !session_chat_browser_e2e || browser_e2e,
        "session-chat browser E2E requires the browser phase"
    );
    let gateway_caddy_e2e = env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() == Ok("1");
    let gateway_service_e2e = env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_E2E").as_deref() == Ok("1");
    let gateway_service_external_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_EXTERNAL_E2E").as_deref() == Ok("1");
    let gateway_service_revocation_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_REVOCATION_E2E").as_deref() == Ok("1");
    let gateway_service_cutover_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CUTOVER_E2E").as_deref() == Ok("1");
    let gateway_service_failed_candidate_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_FAILED_CANDIDATE_E2E").as_deref() == Ok("1");
    let gateway_service_candidate_capacity_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_CANDIDATE_CAPACITY_E2E").as_deref() == Ok("1");
    let gateway_service_rollback_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_ROLLBACK_E2E").as_deref() == Ok("1");
    let gateway_service_log_rpc_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_RPC_E2E").as_deref() == Ok("1");
    let gateway_service_log_guest_e2e =
        env::var("HEPHAESTUS_APP_GATEWAY_SERVICE_LOG_GUEST_E2E").as_deref() == Ok("1");
    assert!(
        !cooking::enabled() || gateway_caddy_e2e,
        "cooking requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !session_chat_e2e || libkrun_e2e,
        "session-chat requires the real libkrun backend"
    );
    assert!(
        !session_chat_e2e || session_chat_browser_e2e || !cooking::enabled(),
        "session-chat acceptance is a standalone golden mode"
    );
    assert!(
        !session_chat_browser_e2e || libkrun_e2e,
        "session-chat browser E2E requires the real libkrun/Caddy fixture"
    );
    assert!(
        !gateway_caddy_e2e || libkrun_e2e,
        "the joined Caddy gateway proof requires the real libkrun backend"
    );
    assert!(
        !gateway_service_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the persistent service proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_external_e2e || gateway_service_e2e,
        "the external persistent-service proof requires the service fixture"
    );
    assert!(
        !gateway_service_revocation_e2e || gateway_service_external_e2e,
        "the service revocation proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_revocation_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the service revocation proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_cutover_e2e || gateway_service_external_e2e,
        "the persistent-service cutover proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_failed_candidate_e2e || gateway_service_external_e2e,
        "the failed-candidate proof requires the external daemon fixture"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || gateway_service_cutover_e2e,
        "the candidate-capacity proof requires the persistent-service cutover fixture"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || !gateway_service_failed_candidate_e2e,
        "the candidate-capacity proof cannot combine with failed-candidate mode"
    );
    assert!(
        !gateway_service_candidate_capacity_e2e || !gateway_service_rollback_e2e,
        "the candidate-capacity proof cannot combine with rollback"
    );
    assert!(
        !gateway_service_rollback_e2e || gateway_service_cutover_e2e,
        "the persistent-service rollback proof requires the cutover fixture"
    );
    assert!(
        !gateway_service_revocation_e2e
            || (!gateway_service_cutover_e2e
                && !gateway_service_rollback_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_candidate_capacity_e2e
                && !gateway_service_log_rpc_e2e
                && !gateway_service_log_guest_e2e),
        "the service revocation proof requires the initial published service mode"
    );
    assert!(
        !gateway_service_log_rpc_e2e || gateway_service_e2e,
        "the service-log RPC proof requires the persistent-service fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || gateway_service_e2e,
        "the guest service-log proof requires the persistent-service fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || (gateway_caddy_e2e && libkrun_e2e),
        "the guest service-log proof requires the real Caddy/libkrun fixture"
    );
    assert!(
        !gateway_service_log_guest_e2e || !gateway_service_log_rpc_e2e,
        "the guest service-log proof cannot combine with seeded service-log RPC rows"
    );
    assert!(
        !cooking_service_build_proof || cooking_build_proof,
        "the Cooking service build proof requires HEPHAESTUS_APP_COOKING_BUILD_PROOF=1"
    );
    assert!(
        !cooking_service_build_proof || cooking::enabled() || session_chat_browser_e2e,
        "the Cooking service build proof requires HEPHAESTUS_APP_COOKING_E2E=1"
    );
    assert!(
        !caddy_tls
            || cooking_service_build_proof
            || session_chat_browser_e2e
            || (cooking_build_proof && cooking::enabled() && gateway_caddy_e2e && libkrun_e2e),
        "Caddy TLS mode requires the published Cooking service proof, browser session-chat proof, or the full Cooking Caddy/libkrun proof"
    );
    assert!(
        !cooking_service_build_proof || (gateway_caddy_e2e && libkrun_e2e),
        "the Cooking service build proof requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !installed_ui_fixture || cooking_service_build_proof || session_chat_browser_e2e,
        "the installed UI fixture requires the Cooking service build proof"
    );
    assert!(
        !installed_ui_fixture || caddy_tls,
        "the installed UI fixture requires Caddy TLS"
    );
    assert!(
        !cooking_service_build_proof
            || (!gateway_service_e2e
                && !gateway_service_external_e2e
                && !gateway_service_cutover_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_candidate_capacity_e2e
                && !gateway_service_rollback_e2e
                && !gateway_service_revocation_e2e
                && !gateway_service_log_rpc_e2e
                && !gateway_service_log_guest_e2e),
        "the Cooking service build proof cannot combine with seeded or alternate service modes"
    );
    assert!(
        !gateway_service_log_guest_e2e || !gateway_service_external_e2e,
        "the guest service-log proof cannot use the external daemon early-return path"
    );
    assert!(
        !gateway_service_log_guest_e2e
            || (!gateway_service_cutover_e2e
                && !gateway_service_failed_candidate_e2e
                && !gateway_service_rollback_e2e),
        "the guest service-log proof requires the initial persistent service revision"
    );
    #[cfg(not(feature = "test-fixtures"))]
    assert!(
        !gateway_service_log_rpc_e2e,
        "the service-log RPC proof requires --features test-fixtures"
    );
    #[cfg(not(feature = "test-fixtures"))]
    assert!(
        !gateway_service_log_guest_e2e,
        "the guest service-log proof requires --features test-fixtures"
    );
    assert!(
        !gateway_service_e2e || !cooking_build_proof,
        "the persistent service proof is incompatible with the Cooking build proof"
    );
    Some(GoldenRunMode {
        workload_phase_timing,
        parent_database_url,
        nats_url,
        parent_before,
        isolated_database,
        database_url,
        libkrun_e2e,
        session_chat_e2e,
        session_chat_denial_probe_e2e,
        session_chat_restart_e2e,
        session_chat_browser_e2e,
        installed_ui_fixture,
        cooking_build_proof,
        release_build_proof,
        cooking_service_build_proof,
        caddy_tls,
        build_timeout,
        cooking_wait_timeout,
        browser_e2e,
        gateway_caddy_e2e,
        gateway_service_e2e,
        gateway_service_external_e2e,
        gateway_service_revocation_e2e,
        gateway_service_cutover_e2e,
        gateway_service_failed_candidate_e2e,
        gateway_service_candidate_capacity_e2e,
        gateway_service_rollback_e2e,
        gateway_service_log_rpc_e2e,
        gateway_service_log_guest_e2e,
    })
}
