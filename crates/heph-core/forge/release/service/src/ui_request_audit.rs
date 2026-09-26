//! Provider-neutral, redacted audit evidence for release-owned UI requests.
//!
//! This boundary contains only closed vocabularies and opaque identities. It
//! has no fields for parent session IDs, bearer digests, credentials, paths,
//! queries, headers, bodies, or provider responses.

#[path = "ui_request_audit/context.rs"]
mod context;
#[path = "ui_request_audit/sink.rs"]
mod sink;
#[path = "ui_request_audit/vocabulary.rs"]
mod vocabulary;

pub use context::{NewUiRequestAuditEvent, UiRequestAuditContext};
pub use sink::{UiRequestAuditError, UiRequestAuditSink};
pub use vocabulary::{
    UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSurface,
};

#[cfg(test)]
mod tests {
    use super::*;
    use identity_domain::RequestId;
    use time::OffsetDateTime;

    #[test]
    fn anonymous_denial_has_no_verified_subject_or_target() {
        let context = UiRequestAuditContext::anonymous();
        let event = NewUiRequestAuditEvent::new_at(
            RequestId::new(),
            UiRequestAuditSurface::Bootstrap,
            UiRequestAuditDecision::Denied,
            UiRequestAuditOutcome::NotAttempted,
            UiRequestAuditReason::Unauthenticated,
            context,
            OffsetDateTime::UNIX_EPOCH,
        );
        assert_eq!(event.context().actor_id(), None);
        assert_eq!(event.context().organization_id(), None);
        assert_eq!(event.context().installation_id(), None);
        assert_eq!(event.context().generation_id(), None);
        assert_eq!(event.context().child_session_id(), None);
        assert_eq!(event.context().gateway_id(), None);
        assert_eq!(event.context().gateway_revision_id(), None);
        assert!(!format!("{event:?}").contains("secret"));
    }

    #[test]
    fn closed_vocabularies_have_stable_values() {
        assert_eq!(UiRequestAuditSurface::Embed.as_str(), "embed");
        assert_eq!(UiRequestAuditDecision::Allowed.as_str(), "allowed");
        assert_eq!(
            UiRequestAuditOutcome::NotAttempted.as_str(),
            "not_attempted"
        );
        assert_eq!(
            UiRequestAuditReason::GenerationMismatch.as_str(),
            "generation_mismatch"
        );
        assert_eq!(
            UiRequestAuditDecision::Undetermined.as_str(),
            "undetermined"
        );
        assert_eq!(UiRequestAuditOutcome::Unknown.as_str(), "unknown");
        assert_eq!(UiRequestAuditReason::Unavailable.as_str(), "unavailable");
    }
}
