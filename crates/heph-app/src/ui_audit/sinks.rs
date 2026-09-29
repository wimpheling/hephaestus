use async_trait::async_trait;
use release_service::{NewUiRequestAuditEvent, UiRequestAuditError, UiRequestAuditSink};

/// Test/composition placeholder only; production wiring must use the worker
/// backed `PostgreSQL` repository.
pub struct NoopAuditSink;

#[async_trait]
impl UiRequestAuditSink for NoopAuditSink {
    async fn append(&self, _event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError> {
        Ok(())
    }
}

/// Capturing sink used only by the external router tests. It is intentionally
/// kept in this draft rather than exported by the production app.
#[derive(Default)]
pub struct CapturingAuditSink {
    pub events: std::sync::Mutex<Vec<NewUiRequestAuditEvent>>,
    pub fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl UiRequestAuditSink for CapturingAuditSink {
    async fn append(&self, event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError> {
        if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(UiRequestAuditError::Unavailable);
        }
        self.events.lock().expect("capturing sink lock").push(event);
        Ok(())
    }
}
