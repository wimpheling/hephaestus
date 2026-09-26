use super::{
    GatewayServiceFailure, GatewayServiceFailureCode, OffsetDateTime, ServiceDiagnostics,
    ServiceInstanceError, ServiceLogBufferHandle, VmError, VmEvent, VmExit, broadcast,
};

pub(super) struct FailureOutcome {
    pub(super) error: ServiceInstanceError,
    pub(super) report: Option<GatewayServiceFailure>,
}

impl FailureOutcome {
    pub(super) const fn shutdown() -> Self {
        Self {
            error: ServiceInstanceError::Shutdown,
            report: None,
        }
    }

    pub(super) const fn startup_timeout() -> Self {
        Self {
            error: ServiceInstanceError::StartupTimeout,
            report: Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::Startup,
                exit_code: None,
                exit_signal: None,
            }),
        }
    }

    pub(super) const fn readiness(error: ServiceInstanceError) -> Self {
        Self {
            error,
            report: Some(GatewayServiceFailure {
                code: GatewayServiceFailureCode::Readiness,
                exit_code: None,
                exit_signal: None,
            }),
        }
    }
}

pub(super) fn map_exit(result: &Result<VmExit, VmError>) -> FailureOutcome {
    let report = result.as_ref().map_or_else(
        |_| failure_for_code(GatewayServiceFailureCode::UnexpectedExit),
        |exit| {
            GatewayServiceFailure::new(
                GatewayServiceFailureCode::UnexpectedExit,
                exit.code,
                exit.signal,
            )
            .ok()
            .or_else(|| failure_for_code(GatewayServiceFailureCode::UnexpectedExit))
        },
    );
    FailureOutcome {
        error: ServiceInstanceError::UnexpectedExit,
        report,
    }
}

pub(super) fn failure_for_startup(error: &ServiceInstanceError) -> Option<GatewayServiceFailure> {
    match error {
        ServiceInstanceError::StartupFailed | ServiceInstanceError::StartupTimeout => {
            failure_for_code(GatewayServiceFailureCode::Startup)
        }
        ServiceInstanceError::Shutdown
        | ServiceInstanceError::InvalidPolicy
        | ServiceInstanceError::UnexpectedExit
        | ServiceInstanceError::NotReady
        | ServiceInstanceError::HealthProbe(_)
        | ServiceInstanceError::CleanupIncomplete => None,
    }
}

pub(super) fn failure_for_code(code: GatewayServiceFailureCode) -> Option<GatewayServiceFailure> {
    GatewayServiceFailure::new(code, None, None).ok()
}

const MAX_DRAIN_EVENTS: usize = 64;

pub(super) fn capture_log_event(logs: Option<&ServiceLogBufferHandle>, event: &VmEvent) {
    let Some(logs) = logs else {
        return;
    };
    if let VmEvent::Log { stream, bytes } = event {
        logs.try_record(*stream, OffsetDateTime::now_utc(), bytes);
    }
}

pub(super) fn observe_event(
    diagnostics: &mut ServiceDiagnostics,
    logs: Option<&ServiceLogBufferHandle>,
    event: Result<VmEvent, broadcast::error::RecvError>,
) -> bool {
    match event {
        Ok(event) => {
            capture_log_event(logs, &event);
            diagnostics.observe(Ok(event))
        }
        Err(error @ broadcast::error::RecvError::Lagged(skipped)) => {
            if let Some(logs) = logs {
                logs.record_provider_lag(skipped);
            }
            diagnostics.observe(Err(error))
        }
        Err(error) => diagnostics.observe(Err(error)),
    }
}

pub(super) fn drain_events(
    events: &mut broadcast::Receiver<vm_trait::VmEvent>,
    diagnostics: &mut ServiceDiagnostics,
    events_open: &mut bool,
    logs: Option<&ServiceLogBufferHandle>,
) {
    for _ in 0..MAX_DRAIN_EVENTS {
        match events.try_recv() {
            Ok(event) => {
                *events_open = observe_event(diagnostics, logs, Ok(event));
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                *events_open = observe_event(
                    diagnostics,
                    logs,
                    Err(broadcast::error::RecvError::Lagged(skipped)),
                );
            }
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => {
                *events_open =
                    observe_event(diagnostics, logs, Err(broadcast::error::RecvError::Closed));
                break;
            }
        }
        if !*events_open {
            break;
        }
    }
}
