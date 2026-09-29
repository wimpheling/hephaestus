use super::*;

/// Runs the standalone session-chat acceptance and restart phases.
#[allow(
    clippy::needless_borrow,
    clippy::needless_return,
    clippy::too_many_arguments
)]
pub async fn run_session_chat_phase(
    pool: &sqlx::PgPool,
    database_url: &str,
    mut running: hephaestus_app::RunningHephaestus,
    root: &Path,
    project_id: ProjectId,
    organization_id: OrganizationId,
    fixture_repository: &PgForgeRepository,
    user_id: UserId,
    browser_oidc_issuer: &str,
    owner_browser_session: &BrowserSessionSid,
    token: &str,
    session_chat_fixture: session_chat::SessionBrokerFixture,
    cooking_wait_timeout: Duration,
    app_config: &AppConfig,
    observer: Option<Arc<support::vm_observer::VmSpecObserver>>,
    nats_url: &str,
) {
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/session-chat")
        .canonicalize()
        .expect("session-chat source root");
    let identity = AuthenticatedIdentity::new(
        user_id,
        browser_oidc_issuer,
        "golden-subject",
        serde_json::json!({}),
        RequestId::new(),
    );
    let rpc_token = |audience: &str| {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({
                "iss": "hephaestus-web-mediator",
                "sub": user_id.to_string(),
                "aud": audience,
                "iat": now,
                "nbf": now,
                "exp": now + 25,
                "jti": uuid::Uuid::new_v4().to_string(),
                "sid": owner_browser_session.to_protocol_string()
            }),
            &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
                b"golden-internal-command-token-with-sufficient-entropy",
            )),
        )
        .expect("sign session-chat mediator token")
    };
    let restart_state = session_chat::exercise(
        pool,
        &database_url,
        &running,
        &root,
        &source_root,
        project_id,
        organization_id,
        &fixture_repository,
        &identity,
        &token,
        &rpc_token,
        session_chat_fixture,
        cooking_wait_timeout,
    )
    .await;
    if let Some(restart_state) = restart_state {
        running
            .shutdown()
            .await
            .expect("session-chat graceful restart shutdown");
        let restart_boundary: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(pool)
            .await
            .expect("session-chat restart database boundary");
        running = Box::pin(restart_application(app_config.clone())).await;
        session_chat::exercise_browser_restart(
            pool,
            &database_url,
            &running,
            &root,
            restart_state,
            restart_boundary,
        )
        .await;
    }
    running
        .shutdown()
        .await
        .expect("session-chat daemon shutdown");
    let observer = observer.expect("session-chat VM observer");
    observer
        .assert_required_kinds(&["agent"])
        .expect("session-chat agent VM observation");
    cleanup_streams(&nats_url).await;
    return;
}
