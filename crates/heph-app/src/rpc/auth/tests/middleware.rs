use super::*;

#[tokio::test]
async fn active_middleware_checks_sid_and_maps_store_results() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let assertion_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = assertion_with_sid(
        &key,
        AUDIENCE,
        user_id.as_uuid(),
        assertion_id,
        now,
        now + 30,
        sid,
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Active,
            calls: Arc::clone(&calls),
        }),
    );
    assert_eq!(
        dispatch_status(state, AUDIENCE, Some(&token)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        *calls.lock().expect("recording store lock"),
        vec![(user_id, sid)]
    );

    let unauthenticated = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Unauthenticated,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(
        dispatch_status(unauthenticated, AUDIENCE, Some(&token)).await,
        StatusCode::UNAUTHORIZED
    );

    let unavailable = MediatorAuthenticationState::new(
        MediatorAuthenticator::new(&key),
        Arc::new(RecordingSessionStore {
            expected_user: user_id,
            expected_sid: sid,
            metadata: session_metadata(user_id),
            outcome: FakeSessionOutcome::Unavailable,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    assert_eq!(
        dispatch_status(unavailable, AUDIENCE, Some(&token)).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn handoff_middleware_failures_are_audited_without_verified_context() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingAuditSink {
        events: Arc::clone(&events),
        fail: false,
    };
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let expired = assertion_with_sid(
        &key,
        super::HANDOFF_AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now - 60,
        now - 30,
        sid,
    );
    let wrong_audience = assertion_with_sid(
        &key,
        AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now,
        now + 30,
        sid,
    );
    let revoked = assertion_with_sid(
        &key,
        super::HANDOFF_AUDIENCE,
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
            outcome: FakeSessionOutcome::Unauthenticated,
            calls: Arc::new(Mutex::new(Vec::new())),
        }),
    )
    .with_ui_request_audit_sink(Arc::new(sink));

    assert_eq!(
        dispatch_status(state.clone(), super::HANDOFF_AUDIENCE, Some(&expired)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        dispatch_status(
            state.clone(),
            super::HANDOFF_AUDIENCE,
            Some(&wrong_audience)
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        dispatch_status(state, super::HANDOFF_AUDIENCE, Some(&revoked)).await,
        StatusCode::UNAUTHORIZED
    );

    let events = events.lock().expect("audit sink lock");
    assert_eq!(events.len(), 3);
    for event in events.iter() {
        assert_eq!(event.surface(), UiRequestAuditSurface::HandoffIssue);
        assert_eq!(event.decision(), UiRequestAuditDecision::Denied);
        assert_eq!(event.outcome(), UiRequestAuditOutcome::NotAttempted);
        assert_eq!(event.reason(), UiRequestAuditReason::Unauthenticated);
        assert_eq!(event.context().actor_id(), None);
        assert_eq!(event.context().installation_id(), None);
    }
    assert_ne!(events[0].request_id(), events[1].request_id());
    drop(events);
}

#[tokio::test]
async fn handoff_audit_failure_preserves_middleware_status() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let expired = assertion_with_sid(
        &key,
        super::HANDOFF_AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now - 60,
        now - 30,
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
    )
    .with_ui_request_audit_sink(Arc::new(RecordingAuditSink {
        events: Arc::new(Mutex::new(Vec::new())),
        fail: true,
    }));
    assert_eq!(
        dispatch_status(state, super::HANDOFF_AUDIENCE, Some(&expired)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn hanging_handoff_audit_sink_is_bounded_without_changing_status() {
    let key = mediator_signing_key(TOKEN);
    let user_id = UserId::new();
    let sid = BrowserSessionSid::new();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let expired = assertion_with_sid(
        &key,
        super::HANDOFF_AUDIENCE,
        user_id.as_uuid(),
        Uuid::new_v4(),
        now - 60,
        now - 30,
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
    )
    .with_ui_request_audit_sink(Arc::new(HangingAuditSink));
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        dispatch_status(state, super::HANDOFF_AUDIENCE, Some(&expired)),
    )
    .await
    .expect("handoff audit timeout must be bounded");
    assert_eq!(response, StatusCode::UNAUTHORIZED);
}
