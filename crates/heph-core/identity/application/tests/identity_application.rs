//! Provider-neutral identity application contract tests.

use async_trait::async_trait;
use identity_application::{
    IdempotentIdentityResolver, IdentityApplication, ResolveIdentity, ResolveIdentityError,
    ResolveVerifiedIdentity, ResolvedIdentity,
};
use identity_domain::{RequestId, UserId};
use std::sync::{Arc, Mutex};

struct RecordingResolver {
    result: Mutex<Option<Result<ResolvedIdentity, ResolveIdentityError>>>,
    observed: Mutex<Vec<ResolveVerifiedIdentity>>,
}

#[async_trait]
impl IdempotentIdentityResolver for RecordingResolver {
    async fn resolve_verified_identity(
        &self,
        request: ResolveVerifiedIdentity,
    ) -> Result<ResolvedIdentity, ResolveIdentityError> {
        self.observed
            .lock()
            .expect("recording resolver observations")
            .push(request);
        self.result
            .lock()
            .expect("recording resolver result")
            .take()
            .expect("one configured resolver result")
    }
}

#[tokio::test]
async fn builds_exact_verified_claims_and_delegates_to_the_injected_port() {
    let expected = ResolvedIdentity {
        user_id: UserId::new(),
        display_name: String::from("Canonical Actor"),
        idempotency_id: RequestId::new(),
    };
    let resolver = Arc::new(RecordingResolver {
        result: Mutex::new(Some(Ok(expected.clone()))),
        observed: Mutex::new(Vec::new()),
    });
    let application = IdentityApplication::new(resolver.clone());
    let request_id = RequestId::new();
    let idempotency_seed = [7_u8; 32];

    let actual = application
        .resolve_identity(ResolveIdentity {
            request_id,
            idempotency_seed,
            issuer: String::from("https://issuer.example"),
            subject: String::from("actor"),
            display_name: String::from("Asserted Actor"),
            email: String::from("actor@example.invalid"),
            email_verified: true,
        })
        .await
        .expect("resolved fake identity");

    assert_eq!(actual, expected);
    let observed = resolver
        .observed
        .lock()
        .expect("recording resolver observations");
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].request_id, request_id);
    assert_eq!(observed[0].idempotency_seed, idempotency_seed);
    assert_eq!(observed[0].verified.issuer, "https://issuer.example");
    assert_eq!(observed[0].verified.subject, "actor");
    assert_eq!(
        observed[0].verified.claims,
        serde_json::json!({
            "email": "actor@example.invalid",
            "email_verified": true,
            "name": "Asserted Actor",
        })
    );
    drop(observed);
}
