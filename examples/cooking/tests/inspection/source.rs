use super::{emit_publication_diagnostic, get_json};
use identity_domain::BrowserSessionSid;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) async fn inspect_https(
    pool: &PgPool,
    run_id: Uuid,
    provenance: &Value,
    expected_https_uses: usize,
) {
    let uses = provenance["httpsUses"]
        .as_array()
        .expect("visible HTTPS evidence");
    assert_eq!(uses.len(), expected_https_uses);
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
    let expected_rules = if expected_https_uses == 2 {
        std::collections::BTreeSet::from(["00000000-0000-0000-0000-000000000004"])
    } else {
        std::collections::BTreeSet::from([
            "00000000-0000-0000-0000-000000000003",
            "00000000-0000-0000-0000-000000000004",
        ])
    };
    assert_eq!(rules, expected_rules);
}

pub(crate) async fn inspect_source(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    client: &reqwest::Client,
    owner: Uuid,
    run_id: Uuid,
    event_id: Uuid,
    owner_browser_session: BrowserSessionSid,
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
    assert_eq!(
        source.1, event_id,
        "inspection must resolve the requested event"
    );
    let gateway = get_json(
        client,
        running,
        owner,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        json!({"gatewayId":{"value":source.2.to_string()}}),
        owner_browser_session,
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
        owner_browser_session,
    )
    .await;
    let publication = publications
        .get("publications")
        .and_then(Value::as_array)
        .and_then(|publications| {
            publications
                .iter()
                .find(|publication| publication["runId"]["value"] == run_id.to_string())
        });
    let Some(publication) = publication else {
        emit_publication_diagnostic(pool, &publications, source.2, event_id, run_id).await;
        panic!("accepted invocation to run");
    };
    assert_eq!(publication["eventId"]["value"], source.1.to_string());
    assert_eq!(publication["runOutcome"], "succeeded");
    let instance = get_json(
        client,
        running,
        owner,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        json!({"instanceId":{"value":source.0.to_string()}}),
        owner_browser_session,
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
