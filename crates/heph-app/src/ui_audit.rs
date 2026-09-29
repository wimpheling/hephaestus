//! Audited UI-origin handler boundary for the external HTTP draft.
//!
//! Only the closed release-service audit vocabulary crosses this helper. It
//! deliberately has no request path, query, header, body, handoff secret, or
//! parent-session field.

#[path = "ui_audit/context.rs"]
mod context;
#[path = "ui_audit/recorder.rs"]
mod recorder;
#[cfg(test)]
#[path = "ui_audit/sinks.rs"]
mod sinks;

use std::time::Duration;

/// Small independent budget for one audit append. It is intentionally shorter
/// than the total UI operation deadline and is applied on every path,
/// including denials outside the inner handler.
pub const DEFAULT_AUDIT_APPEND_TIMEOUT: Duration = Duration::from_millis(250);

pub use context::{
    UiRequestCorrelation, correlation_id, reason_for_bootstrap_error, reason_for_content_error,
    verified_context,
};
pub use recorder::UiAuditRecorder;
#[cfg(test)]
pub use sinks::{CapturingAuditSink, NoopAuditSink};

#[cfg(test)]
mod tests {
    use super::{UiRequestCorrelation, correlation_id};
    use axum::http::Extensions;
    use identity_domain::RequestId;

    #[test]
    fn inner_correlation_reader_reuses_outer_request_id() {
        let request_id = RequestId::new();
        let mut extensions = Extensions::new();
        extensions.insert(UiRequestCorrelation(request_id));
        assert_eq!(correlation_id(&extensions), request_id);
    }
}
