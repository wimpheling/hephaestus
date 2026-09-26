use super::{
    DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT, Duration, FailureReason, GatewayServiceFailure,
    GatewayServiceFailureCode, GatewayServiceIdentity, Instant, LogWriter, ServiceInstanceError,
    ServiceLogWriterPoll, WorkerFuture, time,
};

enum WorkerLogStep {
    Worker(Result<(), ServiceInstanceError>),
    Log(ServiceLogWriterPoll),
}

async fn select_worker_or_log(worker: &mut WorkerFuture, writer: &mut LogWriter) -> WorkerLogStep {
    tokio::select! {
        result = worker.as_mut() => WorkerLogStep::Worker(result),
        poll = writer.poll() => WorkerLogStep::Log(poll),
    }
}

async fn finish_worker_with_logs(
    mut writer: Option<LogWriter>,
    result: Result<(), ServiceInstanceError>,
) -> Result<(), ServiceInstanceError> {
    if let Some(mut writer) = writer.take() {
        let identity = writer.lease().identity;
        writer.shutdown();
        let deadline = Instant::now()
            .checked_add(DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT)
            .unwrap_or_else(Instant::now);
        report_log_flush(identity, writer.final_flush(deadline).await);
    }
    result
}

/// Drives one worker and its optional writer without detaching a lifecycle
/// task. The parent coordinator remains responsible for lease/cancellation
/// signals while this future is selected alongside them.
pub async fn drive_worker_with_logs(
    mut worker: WorkerFuture,
    mut writer: LogWriter,
) -> Result<(), ServiceInstanceError> {
    let mut next_poll = Box::pin(time::sleep(Duration::ZERO));
    loop {
        tokio::select! {
            result = &mut worker => return finish_worker_with_logs(Some(writer), result).await,
            () = &mut next_poll => {
                match select_worker_or_log(&mut worker, &mut writer).await {
                    WorkerLogStep::Worker(result) => {
                        return finish_worker_with_logs(Some(writer), result).await;
                    }
                    WorkerLogStep::Log(ServiceLogWriterPoll::Terminated { .. }) => {
                        let result = worker.await;
                        return finish_worker_with_logs(Some(writer), result).await;
                    }
                    WorkerLogStep::Log(_) => {
                        next_poll.as_mut().reset(Instant::now() + Duration::from_millis(250));
                    }
                }
            }
        }
    }
}

fn report_log_flush(identity: GatewayServiceIdentity, flush: crate::ServiceLogWriterFlush) {
    if !flush.complete {
        tracing::warn!(
            gateway_id = %identity.gateway_id,
            revision_id = %identity.revision_id,
            instance_id = %identity.instance_id,
            unflushed_chunks = flush.unflushed_chunks,
            unflushed_bytes = flush.unflushed_bytes,
            loss_chunks = flush.unflushed_loss.total_chunks(),
            loss_bytes = flush.unflushed_loss.total_bytes(),
            provider_lagged_events = flush.unflushed_loss.provider_lagged_events,
            terminal = flush.terminal_error.is_some(),
            "service log writer flush incomplete"
        );
    }
}
pub(super) fn failure_for_reason(reason: FailureReason) -> Option<GatewayServiceFailure> {
    let code = match reason {
        FailureReason::Preparation => GatewayServiceFailureCode::Preparation,
        FailureReason::StartupDeadline => GatewayServiceFailureCode::Startup,
        FailureReason::Health => GatewayServiceFailureCode::Health,
        FailureReason::CleanupIncomplete => GatewayServiceFailureCode::Cleanup,
        FailureReason::Runtime
        | FailureReason::Ownership
        | FailureReason::Cancelled
        | FailureReason::LeaseLost
        | FailureReason::TargetUnavailable
        | FailureReason::Drained
        | FailureReason::DrainDeadline => return None,
    };
    GatewayServiceFailure::new(code, None, None).ok()
}

pub(super) fn cleanup_report(
    physical: bool,
    report: Option<GatewayServiceFailure>,
) -> Option<GatewayServiceFailure> {
    report.or_else(|| {
        if physical {
            None
        } else {
            failure_for_reason(FailureReason::CleanupIncomplete)
        }
    })
}
