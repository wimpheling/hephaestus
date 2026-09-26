use super::*;

#[tokio::test]
async fn active_authentication_retains_exact_store_session_id() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let metadata = session_metadata(user_id);
    let expected_session_id = metadata.id();
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata,
            outcome: FakeSessionOutcome::Active,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = assertion_with_sid(
        &key,
        AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now,
        now + 30,
        sid,
    );

    let authenticated = state
        .authenticate_active(&headers(&token), AUDIENCE)
        .await
        .expect("active session should authenticate");
    assert_eq!(authenticated.parent_session_id, Some(expected_session_id));
}

#[tokio::test]
async fn revoke_skips_active_lookup_but_keeps_signed_audience_and_sid_checks() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Unavailable,
            calls: Arc::clone(&calls),
        }),
    );
    let token = assertion_with_sid(
        &key,
        REVOKE_AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now,
        now + 30,
        sid,
    );
    let signed = state
        .authenticate_signed(&headers(&token), REVOKE_AUDIENCE)
        .expect("signed revoke assertion should authenticate");
    assert_eq!(signed.parent_session_id, None);
    assert_eq!(
        dispatch_status(state, REVOKE_AUDIENCE, Some(&token)).await,
        StatusCode::NO_CONTENT
    );
    assert!(calls.lock().expect("recording store lock").is_empty());

    let wrong_audience = assertion_with_sid(
        &key,
        AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now,
        now + 30,
        sid,
    );
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Active,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(
        dispatch_status(state, REVOKE_AUDIENCE, Some(&wrong_audience)).await,
        StatusCode::UNAUTHORIZED
    );
    let sidless = assertion_without_sid(
        &key,
        REVOKE_AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now,
        now + 30,
    );
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Active,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(
        dispatch_status(state, REVOKE_AUDIENCE, Some(&sidless)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn only_exact_bootstrap_paths_skip_the_session_gate_and_debug_redacts_sid() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let store = Arc::new(RecordingSessionStore {
        expected_user: user_id,
        expected_sid: sid,
        metadata: session_metadata(user_id),
        outcome: FakeSessionOutcome::Unavailable,
        calls: Arc::new(Mutex::new(Vec::new())),
    });
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::clone(&store) as Arc<dyn BrowserSessionStore>,
    );
    assert_eq!(
        dispatch_status(state, BOOTSTRAP_AUDIENCE, None).await,
        StatusCode::NO_CONTENT
    );
    assert!(store.calls.lock().expect("recording store lock").is_empty());

    let nearby = "/hephaestus.identity.v1.IdentityService/ResolveIdentity/extra";
    let token = assertion_with_sid(
        &key,
        nearby,
        user_id.as_uuid(),
        Uuid::new_v4(),
        OffsetDateTime::now_utc().unix_timestamp(),
        OffsetDateTime::now_utc().unix_timestamp() + 30,
        sid,
    );
    let nearby_state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::clone(&store) as Arc<dyn BrowserSessionStore>,
    );
    assert_eq!(
        dispatch_status(nearby_state, nearby, Some(&token)).await,
        StatusCode::SERVICE_UNAVAILABLE
    );

    let active_calls = Arc::new(Mutex::new(Vec::new()));
    let active_store = Arc::new(RecordingSessionStore {
        expected_user: user_id,
        expected_sid: sid,
        metadata: session_metadata(user_id),
        outcome: FakeSessionOutcome::Active,
        calls: active_calls,
    });
    let active_state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::clone(&active_store) as Arc<dyn BrowserSessionStore>,
    );
    let app =
        Router::new()
            .fallback(inspect_extensions)
            .layer(axum::middleware::from_fn_with_state(
                active_state,
                super::mediator_identity_middleware,
            ));
    let token = assertion_with_sid(
        &key,
        AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        OffsetDateTime::now_utc().unix_timestamp(),
        OffsetDateTime::now_utc().unix_timestamp() + 30,
        sid,
    );
    let mut request = Request::builder().uri(AUDIENCE);
    request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    assert_eq!(
        app.oneshot(request.body(Body::empty()).expect("test request"))
            .await
            .expect("middleware response")
            .status(),
        StatusCode::NO_CONTENT
    );
}
