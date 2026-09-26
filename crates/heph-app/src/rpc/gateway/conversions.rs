use crate::rpc::{RpcError, into_connect_error};
use gateway_postgres::GatewayManagementError;
use rpc_proto::messages::hephaestus::{
    common::v1::OpaqueId,
    gateway::v1::{
        GatewayIngress, GatewayIngressOutcome, GatewayLifecycle, GatewayMailboxBinding,
        GatewayMailboxPublication, GatewayRevision, GatewayRoute, GatewayServiceDeclaration,
        GatewayServiceLogCaptureMode, GatewaySummary,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) const fn map_error(error: &GatewayManagementError) -> RpcError {
    match error {
        GatewayManagementError::Denied => RpcError::PermissionDenied,
        GatewayManagementError::NotFound => RpcError::NotFound,
        GatewayManagementError::InvalidArgument => RpcError::InvalidArgument,
        GatewayManagementError::Conflict => RpcError::FailedPrecondition,
        GatewayManagementError::Unavailable | GatewayManagementError::Persistence(_) => {
            RpcError::Unavailable
        }
    }
}

pub(super) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}

pub(super) fn summary(value: gateway_postgres::GatewayManagementSummary) -> GatewaySummary {
    GatewaySummary {
        id: opaque(value.id).into(),
        project_id: opaque(value.project_id).into(),
        repository_id: opaque(value.repository_id).into(),
        name: value.name,
        lifecycle: lifecycle_proto(&value.lifecycle).into(),
        active_revision_id: value.active_revision_id.map(opaque).into(),
        desired_service_revision_id: value.desired_service_revision_id.map(opaque).into(),
        updated_at: timestamp(value.updated_at).into(),
        ..Default::default()
    }
}

pub(super) fn revision(value: gateway_postgres::GatewayManagementRevision) -> GatewayRevision {
    GatewayRevision {
        id: opaque(value.id).into(),
        release_id: value.release_id.map(opaque).into(),
        release_agent_id: value.release_agent_id.map(opaque).into(),
        handler_contract: value.handler_contract,
        exposure: value.exposure,
        secret_slots: value.secret_slots,
        mailbox_slots: value.mailbox_slots,
        service: value
            .service
            .map(|service| GatewayServiceDeclaration {
                loopback_port: u32::from(service.loopback_port),
                readiness_path: service.readiness_path.as_str().to_owned(),
                health_path: service.health_path.as_str().to_owned(),
                log_capture_mode: service_log_capture_mode(service.log_capture_mode.as_str())
                    .into(),
                ..Default::default()
            })
            .into(),
        created_at: timestamp(value.created_at).into(),
        routes: value.routes.into_iter().map(route).collect(),
        ..Default::default()
    }
}

fn service_log_capture_mode(value: &str) -> GatewayServiceLogCaptureMode {
    match value {
        "disabled" => GatewayServiceLogCaptureMode::GATEWAY_SERVICE_LOG_CAPTURE_MODE_DISABLED,
        "application" => GatewayServiceLogCaptureMode::GATEWAY_SERVICE_LOG_CAPTURE_MODE_APPLICATION,
        _ => GatewayServiceLogCaptureMode::GATEWAY_SERVICE_LOG_CAPTURE_MODE_UNSPECIFIED,
    }
}

fn route(value: gateway_postgres::GatewayManagementRoute) -> GatewayRoute {
    GatewayRoute {
        id: opaque(value.id).into(),
        path: value.path,
        methods: value.methods,
        enabled: value.enabled,
        ..Default::default()
    }
}

pub(super) fn ingress(value: &gateway_postgres::GatewayIngressSummary) -> GatewayIngress {
    GatewayIngress {
        id: opaque(value.id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        gateway_route_id: opaque(value.gateway_route_id).into(),
        outcome: ingress_outcome(&value.outcome).into(),
        accepted_at: timestamp(value.accepted_at).into(),
        completed_at: value.completed_at.map(timestamp).into(),
        ..Default::default()
    }
}

pub(super) fn mailbox_binding(
    value: gateway_postgres::GatewayMailboxBindingSummary,
) -> GatewayMailboxBinding {
    GatewayMailboxBinding {
        id: opaque(value.id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        mailbox_id: opaque(value.mailbox_id).into(),
        slot_key: value.slot_key,
        producer_id: value.producer_id,
        grant_id: opaque(value.grant_id).into(),
        grant_status: value.grant_status,
        created_at: timestamp(value.created_at).into(),
        granted_at: timestamp(value.granted_at).into(),
        revoked_at: value.revoked_at.map(timestamp).into(),
        ..Default::default()
    }
}

pub(super) fn mailbox_publication(
    value: gateway_postgres::GatewayMailboxPublicationSummary,
) -> GatewayMailboxPublication {
    GatewayMailboxPublication {
        id: opaque(value.id).into(),
        invocation_id: opaque(value.invocation_id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        binding_id: value.binding_id.map(opaque).into(),
        grant_id: value.grant_id.map(opaque).into(),
        mailbox_id: value.mailbox_id.map(opaque).into(),
        event_id: value.event_id.map(opaque).into(),
        slot_key: value.slot_key,
        outcome: value.outcome,
        accepted_at: timestamp(value.accepted_at).into(),
        settled_at: timestamp(value.settled_at).into(),
        authorization_snapshot_id: value.authorization_snapshot_id.map(opaque).into(),
        snapshot_binding_ordinal: value
            .snapshot_binding_ordinal
            .and_then(|ordinal| u32::try_from(ordinal).ok()),
        delivery_disposition: value.delivery_disposition.unwrap_or_default(),
        delivery_attempt_count: value
            .delivery_attempt_count
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or_default(),
        delivery_terminal_at: value.delivery_terminal_at.map(timestamp).into(),
        delivery_attempt_id: value.delivery_attempt_id.map(opaque).into(),
        run_id: value.run_id.map(opaque).into(),
        run_state: value.run_state.unwrap_or_default(),
        run_outcome: value.run_outcome.unwrap_or_default(),
        ..Default::default()
    }
}

pub(super) fn lifecycle(value: GatewayLifecycle) -> Result<&'static str, connectrpc::ConnectError> {
    match value {
        GatewayLifecycle::Enabled => Ok("enabled"),
        GatewayLifecycle::Paused => Ok("paused"),
        GatewayLifecycle::Removed => Ok("removed"),
        GatewayLifecycle::Unspecified => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}

fn lifecycle_proto(value: &str) -> GatewayLifecycle {
    match value {
        "enabled" => GatewayLifecycle::Enabled,
        "paused" => GatewayLifecycle::Paused,
        "removed" => GatewayLifecycle::Removed,
        _ => GatewayLifecycle::Unspecified,
    }
}

fn ingress_outcome(value: &str) -> GatewayIngressOutcome {
    match value {
        "accepted" => GatewayIngressOutcome::Accepted,
        "completed" => GatewayIngressOutcome::Completed,
        "failed" => GatewayIngressOutcome::Failed,
        "timed_out" => GatewayIngressOutcome::TimedOut,
        "rejected" => GatewayIngressOutcome::Rejected,
        _ => GatewayIngressOutcome::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::revision;
    use buffa::Message as _;
    use gateway_domain::{
        GatewayServiceConfig, HTTP_HANDLER_CONTRACT_V1, HTTP_SERVICE_HANDLER_CONTRACT_V1,
        ServiceLogCaptureMode, ServiceProbePath,
    };
    use rpc_proto::messages::hephaestus::gateway::v1::{
        GatewayRevision, GatewayServiceLogCaptureMode,
    };
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn service_config(mode: ServiceLogCaptureMode) -> GatewayServiceConfig {
        GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/readyz").expect("valid readiness path"),
            ServiceProbePath::parse("/healthz").expect("valid health path"),
        )
        .expect("valid service config")
        .with_log_capture_mode(mode)
    }

    fn management_revision(service: Option<GatewayServiceConfig>) -> GatewayRevision {
        revision(gateway_postgres::GatewayManagementRevision {
            id: Uuid::nil(),
            release_id: None,
            release_agent_id: None,
            handler_contract: if service.is_some() {
                HTTP_SERVICE_HANDLER_CONTRACT_V1
            } else {
                HTTP_HANDLER_CONTRACT_V1
            }
            .to_owned(),
            service,
            exposure: "private".to_owned(),
            secret_slots: Vec::new(),
            mailbox_slots: Vec::new(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            routes: Vec::new(),
        })
    }

    #[test]
    fn revision_preserves_service_declaration_and_log_modes() {
        for (mode, expected_mode) in [
            (
                ServiceLogCaptureMode::Disabled,
                GatewayServiceLogCaptureMode::Disabled,
            ),
            (
                ServiceLogCaptureMode::Application,
                GatewayServiceLogCaptureMode::Application,
            ),
        ] {
            let service = management_revision(Some(service_config(mode)))
                .service
                .into_option()
                .expect("service revision must expose its declaration");
            assert_eq!(service.loopback_port, 8080);
            assert_eq!(service.readiness_path, "/readyz");
            assert_eq!(service.health_path, "/healthz");
            assert_eq!(service.log_capture_mode, expected_mode);
        }
    }

    #[test]
    fn service_declaration_survives_protobuf_roundtrip() {
        let encoded = management_revision(Some(service_config(ServiceLogCaptureMode::Application)))
            .encode_to_vec();
        let decoded = GatewayRevision::decode_from_slice(&encoded)
            .expect("generated protobuf must decode its own service declaration");
        let service = decoded
            .service
            .into_option()
            .expect("roundtrip must preserve service presence");
        assert_eq!(service.loopback_port, 8080);
        assert_eq!(service.readiness_path, "/readyz");
        assert_eq!(service.health_path, "/healthz");
        assert_eq!(
            service.log_capture_mode,
            GatewayServiceLogCaptureMode::Application
        );
    }

    #[test]
    fn stateless_revision_omits_service_declaration() {
        assert!(management_revision(None).service.is_unset());
    }
}
