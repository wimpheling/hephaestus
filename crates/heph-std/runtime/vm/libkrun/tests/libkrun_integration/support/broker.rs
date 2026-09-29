use std::sync::atomic::{AtomicUsize, Ordering};

use gateway_edge::{
    GatewayEdgeError, GatewayServiceIdentity, GatewayServiceLaunch, GatewayServiceLaunchRequest,
    GatewayServiceLaunchResolver,
};
use secret_broker::{BrokerExecutor, WireBrokerRequest, WireBrokerResponse, WireBrokerStatus};
use vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES;

/// Host fixture that proves the released guest client forwarded only the exact
/// runtime bearer and canonical broker request across the private vsock path.
pub struct IntegrationBroker {
    pub credential: [u8; RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    pub session_id: uuid::Uuid,
}

pub struct IntegrationServiceResolver {
    pub expected: GatewayServiceIdentity,
    pub cleanups: AtomicUsize,
}

#[async_trait::async_trait]
impl GatewayServiceLaunchResolver for IntegrationServiceResolver {
    async fn resolve_service_launch(
        &self,
        _: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }

    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        assert_eq!(identity, self.expected);
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[async_trait::async_trait]
impl BrokerExecutor for IntegrationBroker {
    async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse {
        assert_eq!(request.credential, self.credential);
        assert_eq!(request.run_id.as_uuid(), self.session_id);
        assert_eq!(request.slot, "model");
        assert_eq!(request.destination, "api.example.test");
        assert_eq!(request.operation, "https_v1");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("canonical brokered HTTPS body");
        assert_eq!(body["rule_id"], "00000000-0000-0000-0000-000000000002");
        assert_eq!(body["method"], "get");
        assert_eq!(body["path_and_query"], "/v1/probe");
        assert_eq!(body["headers"], serde_json::json!([]));
        assert_eq!(body["body"], serde_json::json!([]));
        WireBrokerResponse {
            status: WireBrokerStatus::Succeeded,
            body: b"ok".to_vec(),
        }
    }
}
