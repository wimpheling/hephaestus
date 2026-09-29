// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
#[cfg(test)]
mod deferred_event_tests {
    use super::{DeferredEventProjection, deferred_event_is_terminal_success};
    use uuid::Uuid;

    #[test]
    fn materialized_run_is_not_complete_while_delivery_is_eligible() {
        let candidate_revision_id = Uuid::new_v4();
        let projection: DeferredEventProjection = (
            String::from("eligible"),
            Some(Uuid::new_v4()),
            Some(candidate_revision_id),
            Some(String::from("queued")),
            None,
        );

        assert!(!deferred_event_is_terminal_success(
            &projection,
            candidate_revision_id
        ));
        // Cleanup commits before the mailbox completion observer settles the
        // delivery. This intermediate state must keep the real poll waiting.
        let cleaned_projection: DeferredEventProjection = (
            String::from("running"),
            projection.1,
            projection.2,
            Some(String::from("cleaned_up")),
            Some(String::from("succeeded")),
        );
        assert!(!deferred_event_is_terminal_success(
            &cleaned_projection,
            candidate_revision_id
        ));
    }

    #[test]
    fn only_delivered_cleaned_success_is_complete() {
        let candidate_revision_id = Uuid::new_v4();
        let projection: DeferredEventProjection = (
            String::from("delivered"),
            Some(Uuid::new_v4()),
            Some(candidate_revision_id),
            Some(String::from("cleaned_up")),
            Some(String::from("succeeded")),
        );

        assert!(deferred_event_is_terminal_success(
            &projection,
            candidate_revision_id
        ));
    }
}
