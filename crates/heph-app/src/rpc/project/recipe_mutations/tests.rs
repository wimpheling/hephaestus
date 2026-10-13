mod input;
use super::{INSTALL, RESUME, authentication};
use crate::rpc::project::get_recipe_deployment::tests::authentication::{dispatch_for, token};
use axum::http::StatusCode;
use identity_domain::{BrowserSessionSid, UserId};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn mutations_require_distinct_actual_audiences_and_current_session() {
    let user = UserId::new();
    let sid = BrowserSessionSid::new();
    let calls = Arc::new(AtomicUsize::new(0));
    for (audience, wrong) in [(INSTALL, RESUME), (RESUME, INSTALL)] {
        let incorrect = token(user, wrong, false, sid);
        assert_eq!(
            dispatch_for(audience, Some(&incorrect), true, user, sid, calls.clone()).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            dispatch_for(audience, None, true, user, sid, calls.clone()).await,
            StatusCode::UNAUTHORIZED
        );
        let valid = token(user, audience, false, sid);
        assert_eq!(
            dispatch_for(audience, Some(&valid), false, user, sid, calls.clone()).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            dispatch_for(audience, Some(&valid), true, user, sid, calls.clone()).await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
}
#[test]
fn both_methods_share_install_logical_identity_without_cross_actor_takeover() {
    let user = UserId::new();
    let install = authentication::logical_key(user, "same-key");
    assert_eq!(
        install,
        crate::rpc::request::derive_idempotency_id(user.as_uuid().as_bytes(), INSTALL, "same-key")
    );
    assert_ne!(
        install,
        crate::rpc::request::derive_idempotency_id(user.as_uuid().as_bytes(), RESUME, "same-key")
    );
    assert_ne!(
        install,
        authentication::logical_key(UserId::new(), "same-key")
    );
}
