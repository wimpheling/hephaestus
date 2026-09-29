// Secret broker test scenarios.
use super::support::*;

#[test]
fn registry_rejects_duplicate_or_unpinned_rules() {
    let rule = outbound_rule();
    let upstream = BrokeredHttpsUpstream {
        rule: rule.clone(),
        addresses: vec!["203.0.113.7".parse().expect("test address")],
    };
    assert!(matches!(
        BrokeredHttpsAdapterRegistry::new(vec![upstream.clone(), upstream]),
        Err(BrokerAdapterError::Rejected)
    ));
    assert!(matches!(
        BrokeredHttpsAdapterRegistry::new(vec![BrokeredHttpsUpstream {
            rule,
            addresses: vec!["127.0.0.1".parse().expect("loopback address")],
        }]),
        Err(BrokerAdapterError::Rejected)
    ));
    assert!(matches!(
        BrokeredHttpsAdapterRegistry::new_with_origin_catalog(
            Vec::new(),
            vec![BrokeredHttpsOrigin {
                origin: String::from("https://api.example.test"),
                addresses: vec!["127.0.0.1".parse().expect("loopback address")],
            }],
        ),
        Err(BrokerAdapterError::Rejected)
    ));
}
#[tokio::test]
async fn generic_https_adapter_rejects_an_upstream_secret_echo() {
    let rule = outbound_rule();
    let placeholder = rule.placeholder();
    let rule_id = rule.id.as_uuid();
    let adapter = BrokeredHttpsAdapter::new(
        rule,
        RecordingHttpsTransport {
            seen: Mutex::new(Vec::new()),
            response: b"upstream echoed real-secret-sentinel".to_vec(),
        },
    )
    .expect("valid adapter");
    let request = BrokeredHttpsRequest {
        rule_id,
        method: BrokeredHttpsMethod::Post,
        path_and_query: String::from("/v1/messages?bounded=true"),
        headers: vec![BrokeredHttpsHeader {
            name: String::from("authorization"),
            value: format!("Bearer {placeholder}"),
        }],
        body: Vec::new(),
    };
    let result = adapter
        .invoke(
            &SecretValue::new("real-secret-sentinel").expect("secret"),
            "api.example.test",
            "https_v1",
            &serde_json::to_vec(&request).expect("request"),
        )
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
}
#[tokio::test]
async fn generic_https_adapter_rejects_ambiguous_or_unbound_requests() {
    let rule = outbound_rule();
    let placeholder = rule.placeholder();
    let adapter =
        BrokeredHttpsAdapter::new(rule, RecordingHttpsTransport::default()).expect("valid adapter");
    let credential = SecretValue::new("real-secret").expect("secret");
    for request in [
        BrokeredHttpsRequest {
            rule_id: uuid::Uuid::new_v4(),
            method: BrokeredHttpsMethod::Get,
            path_and_query: String::from("https://metadata.google.internal/"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: format!("Bearer {placeholder}"),
            }],
            body: Vec::new(),
        },
        BrokeredHttpsRequest {
            rule_id: uuid::Uuid::new_v4(),
            method: BrokeredHttpsMethod::Get,
            path_and_query: String::from("/safe"),
            headers: vec![
                BrokeredHttpsHeader {
                    name: String::from("authorization"),
                    value: format!("Bearer {placeholder}"),
                },
                BrokeredHttpsHeader {
                    name: String::from("authorization"),
                    value: format!("Bearer {placeholder}"),
                },
            ],
            body: Vec::new(),
        },
        BrokeredHttpsRequest {
            rule_id: uuid::Uuid::new_v4(),
            method: BrokeredHttpsMethod::Get,
            path_and_query: String::from("/safe"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: String::from("Bearer different-placeholder"),
            }],
            body: Vec::new(),
        },
    ] {
        let result = adapter
            .invoke(
                &credential,
                "api.example.test",
                "https_v1",
                &serde_json::to_vec(&request).expect("request"),
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    }
    let request = BrokeredHttpsRequest {
        rule_id: uuid::Uuid::new_v4(),
        method: BrokeredHttpsMethod::Get,
        path_and_query: String::from("/safe"),
        headers: vec![BrokeredHttpsHeader {
            name: String::from("authorization"),
            value: format!("Bearer {placeholder}"),
        }],
        body: Vec::new(),
    };
    let result = adapter
        .invoke(
            &credential,
            "other.example.test",
            "https_v1",
            &serde_json::to_vec(&request).expect("request"),
        )
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
}
