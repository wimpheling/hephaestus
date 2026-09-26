use super::{
    Arc, Duration, LogStream, MAX_BUILD_LOG_BYTES, MAX_BUILD_METRICS, Value, VmEvent, broadcast,
    json,
};
use vm_trait::VmExit;

pub(super) async fn collect_execution(
    instance: &Arc<dyn vm_trait::VmInstance>,
    events: &mut broadcast::Receiver<VmEvent>,
    timeout: Duration,
) -> (Option<VmExit>, Vec<Value>, Vec<Value>, bool) {
    let wait = instance.wait();
    tokio::pin!(wait);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut logs = Vec::new();
    let mut metrics = Vec::new();
    let mut log_bytes = 0_usize;
    loop {
        tokio::select! {
            result = &mut wait => {
                drain_execution_events(events, &mut logs, &mut metrics, &mut log_bytes);
                return (result.ok(), logs, metrics, false);
            },
            () = &mut deadline => return (None, logs, metrics, true),
            event = events.recv() => match event {
                Ok(event) => capture_execution_event(
                    event,
                    &mut logs,
                    &mut metrics,
                    &mut log_bytes,
                ),
                Err(
                    broadcast::error::RecvError::Lagged(_)
                    | broadcast::error::RecvError::Closed,
                ) => {}
            }
        }
    }
}

fn drain_execution_events(
    events: &mut broadcast::Receiver<VmEvent>,
    logs: &mut Vec<Value>,
    metrics: &mut Vec<Value>,
    log_bytes: &mut usize,
) {
    loop {
        match events.try_recv() {
            Ok(event) => capture_execution_event(event, logs, metrics, log_bytes),
            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                return;
            }
        }
    }
}

fn capture_execution_event(
    event: VmEvent,
    logs: &mut Vec<Value>,
    metrics: &mut Vec<Value>,
    log_bytes: &mut usize,
) {
    match event {
        VmEvent::Log { stream, bytes } => {
            if let Some(next) = log_bytes.checked_add(bytes.len())
                && next <= MAX_BUILD_LOG_BYTES
            {
                *log_bytes = next;
                logs.push(json!({
                    "stream": match stream {
                        LogStream::Stdout => "stdout",
                        LogStream::Stderr => "stderr",
                        _ => "unknown",
                    },
                    "text": String::from_utf8_lossy(&bytes),
                }));
            }
        }
        VmEvent::Metric(metric) if metrics.len() < MAX_BUILD_METRICS => {
            metrics.push(json!({
                "name": metric.name,
                "value": metric.value,
                "labels": metric.labels,
            }));
        }
        _ => {}
    }
}
