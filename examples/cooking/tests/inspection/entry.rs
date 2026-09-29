use super::{approve, assertion, inspect_https, inspect_source};
use identity_domain::BrowserSessionSid;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

// Keep owner and outsider session inputs explicit so each authorization probe
// remains independently auditable in this acceptance boundary.
#[allow(clippy::too_many_arguments)]
pub async fn inspect(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run_id: Uuid,
    event_id: Uuid,
    approve_result: bool,
    owner_browser_session: BrowserSessionSid,
    outsider: Uuid,
    outsider_browser_session: BrowserSessionSid,
) -> Value {
    inspect_with_https_uses(
        pool,
        running,
        run_id,
        event_id,
        approve_result,
        4,
        owner_browser_session,
        outsider,
        outsider_browser_session,
    )
    .await
}

/// Inspects a run whose controlled fault path may repeat one broker call.
/// The expected count remains explicit so the happy path keeps its exact
/// four-use assertion while retries prove their additional physical use.
// Explicit owner/outsider sessions keep the authorization probes independent.
// Keep the retry expectation and both authenticated session identities explicit
// so the acceptance assertions cannot hide authorization coupling.
#[allow(clippy::too_many_arguments)]
pub async fn inspect_with_https_uses(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run_id: Uuid,
    event_id: Uuid,
    approve_result: bool,
    expected_https_uses: usize,
    owner_browser_session: BrowserSessionSid,
    outsider: Uuid,
    outsider_browser_session: BrowserSessionSid,
) -> Value {
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = 'golden-subject'",
    )
    .bind(super::super::golden_issuer())
    .fetch_one(pool)
    .await
    .expect("golden inspection owner");
    let client = reqwest::Client::new();
    let mut provenance = Value::Null;
    let mut result_run = Value::Null;
    inspect_source(
        pool,
        running,
        &client,
        owner,
        run_id,
        event_id,
        owner_browser_session,
    )
    .await;
    for method in ["GetRun", "GetRunProvenance"] {
        let audience = format!("/hephaestus.run.v1.RunService/{method}");
        let url = format!("http://{}{audience}", running.http_addr());
        let request = json!({"runId":{"value":run_id.to_string()}});
        let response = client
            .post(&url)
            .bearer_auth(assertion(owner, &audience, owner_browser_session))
            .json(&request)
            .send()
            .await
            .expect("owner inspection RPC");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: Value = response.json().await.expect("typed JSON inspection");
        if method == "GetRun" {
            assert_eq!(body["run"]["id"]["value"], run_id.to_string());
            assert!(
                !body["run"]["result"]["commit"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty()
            );
            assert_eq!(body["run"]["gitRef"], "refs/heads/main");
            result_run = body["run"].clone();
        } else {
            provenance = body;
        }
        let outsider = client
            .post(&url)
            .bearer_auth(assertion(outsider, &audience, outsider_browser_session))
            .json(&request)
            .send()
            .await
            .expect("outsider inspection RPC");
        assert_eq!(outsider.status(), reqwest::StatusCode::NOT_FOUND);
        let wrong_audience = client
            .post(&url)
            .bearer_auth(assertion(owner, "/wrong-audience", owner_browser_session))
            .json(&request)
            .send()
            .await
            .expect("wrong-audience inspection RPC");
        assert_eq!(wrong_audience.status(), reqwest::StatusCode::UNAUTHORIZED);
    }
    assert!(
        !provenance["authorizationSnapshotId"]["value"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    inspect_https(pool, run_id, &provenance, expected_https_uses).await;
    assert!(
        !provenance
            .to_string()
            .contains(super::super::BROKERED_E2E_SENTINEL)
    );
    super::super::cooking::assert_no_credentials(&provenance.to_string());
    if approve_result {
        approve(
            pool,
            running,
            &client,
            owner,
            owner_browser_session,
            &result_run,
        )
        .await;
    }
    result_run
}
