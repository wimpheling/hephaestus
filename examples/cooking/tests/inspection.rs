//! Authenticated HTTP inspection of the real mailbox cooking run.

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn inspect(pool: &PgPool, running: &hephaestus_app::RunningHephaestus, run_id: Uuid) {
    let owner: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE issuer = $1 AND subject = 'golden-subject'",
    )
    .bind(super::ISSUER)
    .fetch_one(pool)
    .await
    .expect("golden inspection owner");
    let client = reqwest::Client::new();
    let mut provenance = Value::Null;
    let mut result_run = Value::Null;
    inspect_source(pool, running, &client, owner, run_id).await;
    for method in ["GetRun", "GetRunProvenance"] {
        let audience = format!("/hephaestus.run.v1.RunService/{method}");
        let url = format!("http://{}{audience}", running.http_addr());
        let request = json!({"runId":{"value":run_id.to_string()}});
        let response = client
            .post(&url)
            .bearer_auth(assertion(owner, &audience))
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
            .bearer_auth(assertion(Uuid::new_v4(), &audience))
            .json(&request)
            .send()
            .await
            .expect("outsider inspection RPC");
        assert_eq!(outsider.status(), reqwest::StatusCode::NOT_FOUND);
        let wrong_audience = client
            .post(&url)
            .bearer_auth(assertion(owner, "/wrong-audience"))
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
    inspect_https(pool, run_id, &provenance).await;
    assert!(
        !provenance
            .to_string()
            .contains(super::BROKERED_E2E_SENTINEL)
    );
    approve(pool, running, &client, owner, &result_run).await;
}

async fn inspect_https(pool: &PgPool, run_id: Uuid, provenance: &Value) {
    let uses = provenance["httpsUses"]
        .as_array()
        .expect("visible HTTPS evidence");
    assert_eq!(
        uses.len(),
        4,
        "model and relay each record authorization and substitution"
    );
    let mut rules = std::collections::BTreeSet::new();
    for usage in uses {
        for field in [
            "leaseId",
            "bindingId",
            "secretVersionId",
            "ruleId",
            "requestId",
        ] {
            assert!(
                Uuid::parse_str(usage[field]["value"].as_str().expect("exact opaque ID")).is_ok()
            );
        }
        rules.insert(usage["ruleId"]["value"].as_str().expect("rule ID"));
        let expected: (Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT lease_id,binding_id,secret_version_id FROM brokered_secret_lease_snapshots WHERE run_id=$1 AND rule_id=$2",
        ).bind(run_id).bind(Uuid::parse_str(usage["ruleId"]["value"].as_str().expect("rule")).expect("rule UUID"))
            .fetch_one(pool).await.expect("exact immutable lease evidence");
        assert_eq!(usage["leaseId"]["value"], expected.0.to_string());
        assert_eq!(usage["bindingId"]["value"], expected.1.to_string());
        assert_eq!(usage["secretVersionId"]["value"], expected.2.to_string());
    }
    assert_eq!(
        rules,
        std::collections::BTreeSet::from([
            "00000000-0000-0000-0000-000000000003",
            "00000000-0000-0000-0000-000000000004",
        ])
    );
}

async fn inspect_source(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    run_id: Uuid,
) {
    let source: (Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT run.instance_id,delivery.event_id,revision.gateway_id,lease.id
         FROM runs run JOIN mailbox_delivery_attempts delivery ON delivery.run_id=run.id
         JOIN gateway_mailbox_publications publication ON publication.event_id=delivery.event_id
         JOIN gateway_revisions revision ON revision.id=publication.gateway_revision_id
         JOIN agent_instance_volume_leases lease ON lease.run_id=run.id WHERE run.id=$1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("exact ingress and state chain");
    let gateway = get_json(
        client,
        running,
        owner,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        json!({"gatewayId":{"value":source.2.to_string()}}),
    )
    .await;
    assert!(
        gateway["revisions"]
            .as_array()
            .expect("gateway revisions")
            .iter()
            .any(
                |revision| revision["routes"].as_array().is_some_and(|routes| routes
                    .iter()
                    .any(|route| route["path"] == "/cooking/telegram"
                        || route["path"] == "/gateway/cooking/telegram"))
            )
    );
    let publications = get_json(
        client,
        running,
        owner,
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
        json!({"gatewayId":{"value":source.2.to_string()}}),
    )
    .await;
    let publication = publications["publications"]
        .as_array()
        .expect("publication chain")
        .iter()
        .find(|publication| publication["runId"]["value"] == run_id.to_string())
        .expect("accepted invocation to run");
    assert_eq!(publication["eventId"]["value"], source.1.to_string());
    assert_eq!(publication["runOutcome"], "succeeded");
    let instance = get_json(
        client,
        running,
        owner,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        json!({"instanceId":{"value":source.0.to_string()}}),
    )
    .await;
    let delivery = instance["instance"]["mailboxDeliveries"]
        .as_array()
        .expect("mailbox deliveries")
        .iter()
        .find(|delivery| delivery["eventId"]["value"] == source.1.to_string())
        .expect("event state lease");
    assert_eq!(delivery["leaseId"]["value"], source.3.to_string());
    assert!(
        !delivery["stateVolumeId"]["value"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
}

async fn get_json(
    client: &reqwest::Client,
    running: &hephaestus_app::RunningHephaestus,
    owner: Uuid,
    audience: &str,
    request: Value,
) -> Value {
    let response = client
        .post(format!("http://{}{audience}", running.http_addr()))
        .bearer_auth(assertion(owner, audience))
        .json(&request)
        .send()
        .await
        .expect("authorized provenance query");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "provenance RPC {audience}"
    );
    response.json().await.expect("provenance response")
}

async fn approve(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    run: &Value,
) {
    let audience = "/hephaestus.run.v1.RunService/RequestControl";
    let proposal = run["result"]["proposal"]["id"]["value"]
        .as_str()
        .expect("proposal ID");
    let response = client.post(format!("http://{}{audience}", running.http_addr()))
        .bearer_auth(assertion(owner, audience))
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

fn assertion(user: Uuid, audience: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let signing_key = hephaestus_app::rpc::mediator_signing_key(b"golden-internal-command-token");
    encode(
        &Header::new(Algorithm::HS256),
        &json!({
            "iss":"hephaestus-web-mediator", "sub":user.to_string(),
            "aud":audience, "jti":Uuid::new_v4().to_string(),
            "iat":now, "nbf":now, "exp":now+30,
        }),
        &EncodingKey::from_secret(&signing_key),
    )
    .expect("sign exact inspection assertion")
}
