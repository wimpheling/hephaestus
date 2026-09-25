pub(super) enum GatewayAuditDisposition {
    Denied(release_service::UiRequestAuditReason),
    Failed(release_service::UiRequestAuditReason),
    Unknown,
    Succeeded,
}

pub(super) const fn gateway_audit_disposition(
    disposition: gateway_edge::UiDispatchDisposition,
) -> GatewayAuditDisposition {
    use gateway_edge::{GatewayInvocationOutcome, UiDispatchDisposition};
    match disposition {
        UiDispatchDisposition::ProviderDenied => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unauthorized)
        }
        UiDispatchDisposition::ProviderNotFound => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::NotFound)
        }
        UiDispatchDisposition::ProviderUnavailable => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unavailable)
        }
        UiDispatchDisposition::StructuralInvalid => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::InvalidInput)
        }
        UiDispatchDisposition::AcceptedUiFailure => {
            GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::Unavailable)
        }
        UiDispatchDisposition::Admitted {
            completion_persisted: false,
            ..
        }
        | UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::TimedOut,
            completion_persisted: true,
        } => GatewayAuditDisposition::Unknown,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Completed,
            completion_persisted: true,
        } => GatewayAuditDisposition::Succeeded,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Rejected,
            completion_persisted: true,
        } => GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::Unauthorized),
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Failed,
            completion_persisted: true,
        } => {
            GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::UpstreamFailure)
        }
    }
}
