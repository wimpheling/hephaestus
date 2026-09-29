use identity_domain::BrowserSessionSid;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub(crate) fn assertion(
    user: Uuid,
    audience: &str,
    owner_browser_session: BrowserSessionSid,
) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let signing_key = hephaestus_app::rpc::mediator_signing_key(
        b"golden-internal-command-token-with-sufficient-entropy",
    );
    encode(
        &Header::new(Algorithm::HS256),
        &json!({
            "iss":"hephaestus-web-mediator", "sub":user.to_string(),
            "aud":audience, "jti":Uuid::new_v4().to_string(),
            "iat":now, "nbf":now, "exp":now+30,
            "sid":owner_browser_session.to_protocol_string(),
        }),
        &EncodingKey::from_secret(&signing_key),
    )
    .expect("sign exact inspection assertion")
}

pub(crate) async fn approve(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    owner_browser_session: BrowserSessionSid,
    run: &Value,
) {
    let audience = "/hephaestus.run.v1.RunService/RequestControl";
    let proposal = run["result"]["proposal"]["id"]["value"]
        .as_str()
        .expect("proposal ID");
    let response = client.post(format!("http://{}{audience}", running.http_addr()))
        .bearer_auth(assertion(owner, audience, owner_browser_session))
        .json(&json!({
            "context":{"requestId":{"value":Uuid::new_v4().to_string()}, "idempotencyKey":format!("cooking-approve-{proposal}")},
            "kind":"RUN_CONTROL_KIND_APPROVE_RESULT", "repositoryId":run["repositoryId"],
            "target":{"proposalId":{"value":proposal}}, "reason":"Publish the verified cooking fixture result",
        })).send().await.expect("authorized cooking approval");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: Value = response.json().await.expect("approval receipt");
    let control = Uuid::parse_str(
        receipt["controlRequestId"]["value"]
            .as_str()
            .expect("control ID"),
    )
    .expect("control UUID");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM control_requests WHERE id=$1")
                    .bind(control)
                    .fetch_one(pool)
                    .await
                    .expect("approval disposition");
            assert_ne!(state, "failed", "controlled publication failed");
            if state == "completed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("controlled publication completes");
}

/// Requests approval for a separately inspected proposal. The caller uses
/// this for a stale competing proposal after canonical Git has advanced.
pub async fn approve_for_test(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    run: &Value,
    owner_browser_session: BrowserSessionSid,
) {
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = 'golden-subject'",
    )
    .bind(super::super::golden_issuer())
    .fetch_one(pool)
    .await
    .expect("golden approval owner");
    approve(
        pool,
        running,
        &reqwest::Client::new(),
        owner,
        owner_browser_session,
        run,
    )
    .await;
}
