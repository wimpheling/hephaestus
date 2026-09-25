//! Real external-daemon service release-revocation proof.
//!
//! This module is included by the golden integration test after the external
//! daemon and its immutable service fixture have been created.  It deliberately
//! uses the production release command so publication authority, actor
//! authorization, idempotency, and the service supervisor observe one durable
//! revocation rather than a test-only SQL state change.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use release_domain::{ReleaseCommandKey, ReleaseId};
use release_postgres::ReleaseService;
use sqlx::PgPool;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

/// Revokes the published service release while one real public request is
/// accepted by the external daemon.  Publication revocation cancels the
/// in-flight exchange; it must not be tested as graceful drain behavior.
// Keep the real fixture boundaries explicit; this helper is included only by
// the private golden module, despite its public item visibility here.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn exercise_external_gateway_service_revocation(
    pool: &PgPool,
    fixture: &super::GatewayServiceGoldenFixture,
    daemon: &super::ExternalGoldenDaemon,
    instance_id: Uuid,
    resource_paths: &(PathBuf, PathBuf, PathBuf),
    public_url: &str,
    owner: &AuthenticatedIdentity,
) {
    assert!(
        daemon.child.id().is_some(),
        "external daemon must remain live for release revocation proof"
    );

    let baseline_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count invocations before release revocation hold");
    let started_at: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read release revocation hold start time");
    let fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(instance_id)
    .bind(fixture.gateway_id)
    .bind(fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read release revocation hold fencing token");

    let hold_url = format!(
        "{}/gateway/service/hold?revocation_nonce={}",
        public_url.trim_end_matches('/'),
        Uuid::new_v4()
    );
    let hold = tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("bounded release revocation hold client");
        client
            .get(hold_url)
            .send()
            .await
            .expect("release revocation hold response")
    });
    let invocation_id = super::wait_for_accepted_gateway_service_hold(
        pool,
        fixture,
        instance_id,
        fencing_token,
        started_at,
        baseline_invocations,
    )
    .await;
    assert!(
        !hold.is_finished(),
        "release revocation hold remains in flight after durable acceptance"
    );

    let release_id: Uuid = sqlx::query_scalar(
        "SELECT release_id
           FROM gateway_revisions
          WHERE id = $1 AND gateway_id = $2",
    )
    .bind(fixture.revision_id)
    .bind(fixture.gateway_id)
    .fetch_one(pool)
    .await
    .expect("read immutable service release identity");
    let release_service =
        ReleaseService::new(pool.clone(), std::sync::Arc::new(PostgresMelangeAuthorizer));
    let command_key = ReleaseCommandKey::derive(
        "golden-service-release-revocation",
        &[release_id.as_bytes()],
    );
    release_service
        .revoke(owner, command_key, ReleaseId::from_uuid(release_id))
        .await
        .expect("revoke published service release through production command");
    release_service
        .revoke(owner, command_key, ReleaseId::from_uuid(release_id))
        .await
        .expect("replay release revocation command idempotently");

    let release_state: String = sqlx::query_scalar("SELECT state FROM releases WHERE id = $1")
        .bind(release_id)
        .fetch_one(pool)
        .await
        .expect("read revoked service release state");
    assert_eq!(release_state, "revoked");

    let new_request = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded post-revocation admission client")
        .get(format!(
            "{}/gateway/service/identity?revocation_probe={}",
            public_url.trim_end_matches('/'),
            Uuid::new_v4()
        ))
        .send()
        .await
        .expect("post-revocation admission response");
    assert!(
        !new_request.status().is_success(),
        "revoked release must deny new service admission"
    );
    let post_revoke_invocations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1")
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count invocations after release revocation admission probe");
    // service_admission_binding rejects an unpublished release before the
    // gateway_invocations INSERT, so denial is evidenced by no new row.
    assert_eq!(
        post_revoke_invocations,
        baseline_invocations + 1,
        "post-revocation probe must not create another invocation"
    );

    let hold_response = tokio::time::timeout(Duration::from_secs(15), hold)
        .await
        .expect("revoked hold cancellation deadline")
        .expect("revoked hold task joins");
    assert_eq!(
        hold_response.status(),
        reqwest::StatusCode::BAD_GATEWAY,
        "release revocation cancels the accepted service exchange"
    );
    wait_for_failed_invocation(pool, invocation_id).await;

    super::wait_for_gateway_service_cleaned(pool, fixture, instance_id).await;
    assert!(
        !resource_paths.0.exists(),
        "revoked service VM runtime is cleaned"
    );
    assert!(
        !resource_paths.1.exists(),
        "revoked service cgroup is cleaned"
    );
    assert!(
        !resource_paths.2.exists(),
        "revoked service materializer is cleaned"
    );
}

async fn wait_for_failed_invocation(pool: &PgPool, invocation_id: Uuid) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let outcome: String =
                sqlx::query_scalar("SELECT outcome FROM gateway_invocations WHERE id = $1")
                    .bind(invocation_id)
                    .fetch_one(pool)
                    .await
                    .expect("read revoked hold invocation outcome");
            if outcome == "failed" {
                return;
            }
            assert_eq!(
                outcome, "accepted",
                "revoked hold must not complete or time out before cancellation"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("revoked hold invocation reaches Failed");
}
