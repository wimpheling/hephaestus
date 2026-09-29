use async_trait::async_trait;
// Shared test fixtures and explicit production imports for protocol scenarios.
pub(super) use super::super::*;
pub(super) use crate::broker::{adapter::*, common::*, executor::*, server::*, types::*};
pub(super) use brokered_egress_domain::{
    BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
pub(super) use runtime_types::RunId;
pub(super) use std::sync::Mutex;
pub(super) use tempfile::TempDir;
pub(super) use tokio::net::{TcpListener, UnixStream};
pub(super) use uuid::Uuid;

pub(super) fn outbound_rule() -> BrokeredSecretRule {
    BrokeredSecretRule {
        id: BrokeredSecretRuleId::new(),
        binding_id: Uuid::new_v4(),
        instance_revision_id: Uuid::new_v4(),
        secret_version_id: Uuid::new_v4(),
        destination: Some(ExactHttpsOrigin::parse("https://api.example.test").expect("origin")),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    }
}

pub(super) fn dynamic_request(rule_id: Uuid, destination: &str) -> BrokerRequest {
    BrokerRequest {
        run_id: RunId::new(),
        slot: SecretSlotKey::parse("model").expect("slot"),
        destination: destination.to_owned(),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&BrokeredHttpsRequest {
            rule_id,
            method: BrokeredHttpsMethod::Post,
            path_and_query: String::from("/v1/messages?bounded=true"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: format!("Bearer heph-placeholder:v1:{rule_id}"),
            }],
            body: br#"{"message":"hello"}"#.to_vec(),
        })
        .expect("request"),
    }
}

pub(super) struct RecordingExecutor {
    pub(super) seen: Mutex<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct RecordingHttpsTransport {
    pub(super) seen: Mutex<Vec<BrokeredHttpsHeader>>,
    pub(super) response: Vec<u8>,
}

#[async_trait]
impl PinnedHttpsTransport for RecordingHttpsTransport {
    async fn send(
        &self,
        request: UpstreamHttpsRequest,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        assert_eq!(request.destination(), "api.example.test");
        assert_eq!(request.path_and_query(), "/v1/messages?bounded=true");
        *self.seen.lock().expect("recording lock") = request.headers;
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: if self.response.is_empty() {
                b"ordinary response".to_vec()
            } else {
                self.response.clone()
            },
        })
    }
}

#[async_trait]
impl BrokerExecutor for RecordingExecutor {
    async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse {
        *self.seen.lock().expect("recording lock") = request.credential;
        WireBrokerResponse {
            status: WireBrokerStatus::Succeeded,
            body: b"sanitized".to_vec(),
        }
    }
}
