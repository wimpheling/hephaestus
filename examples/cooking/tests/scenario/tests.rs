// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
#[cfg(test)]
mod rule_copy_tests {
    use super::rewrite_rule_copy_request;

    #[test]
    fn copied_rule_rewrites_candidate_placeholder_after_rotation() {
        let source = "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("source rule UUID");
        let candidate = "22222222-2222-4222-8222-222222222222"
            .parse()
            .expect("candidate rule UUID");
        let body = serde_json::json!({
            "rule_id": candidate,
            "method": "post",
            "path_and_query": "/v1/recipes",
            "headers": [
                {"name": "authorization", "value": format!("Bearer heph-placeholder:v1:{candidate}")},
                {"name": "content-type", "value": "application/json"}
            ],
            "body": []
        });

        let rewritten = rewrite_rule_copy_request(
            &serde_json::to_vec(&body).expect("request JSON"),
            source,
            candidate,
        )
        .expect("candidate request maps to source");
        let rewritten: serde_json::Value =
            serde_json::from_slice(&rewritten).expect("rewritten JSON");
        assert_eq!(rewritten["rule_id"], source.to_string());
        assert_eq!(
            rewritten["headers"][0]["value"],
            format!("Bearer heph-placeholder:v1:{source}")
        );
    }

    #[test]
    fn copied_rule_rejects_a_placeholder_for_another_rule() {
        let source = "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("source rule UUID");
        let candidate = "22222222-2222-4222-8222-222222222222"
            .parse()
            .expect("candidate rule UUID");
        let body = serde_json::json!({
            "rule_id": candidate,
            "headers": [{
                "name": "authorization",
                "value": "Bearer heph-placeholder:v1:33333333-3333-4333-8333-333333333333"
            }]
        });
        assert!(matches!(
            rewrite_rule_copy_request(
                &serde_json::to_vec(&body).expect("request JSON"),
                source,
                candidate,
            ),
            Err(secret_application::BrokerAdapterError::Rejected)
        ));
    }
}

#[cfg(test)]
mod crash_upstream_tests {
    use super::{BASE_COOKING_REQUESTS, EXPECTED_COOKING_REQUESTS};
    use super::{
        MODEL_ROTATED_SENTINEL, MODEL_SENTINEL, credential_class, expected_relay_ledger,
        expected_upstream_requests,
    };

    #[test]
    fn crash_mode_completion_budget_is_six_for_both_adapters() {
        assert_eq!(expected_upstream_requests(true, true, true), 6);
        assert_eq!(expected_upstream_requests(true, true, false), 6);
        assert_eq!(expected_upstream_requests(true, false, true), 6);
        assert_eq!(expected_upstream_requests(true, false, false), 6);
        assert_eq!(
            expected_upstream_requests(false, true, true),
            EXPECTED_COOKING_REQUESTS
        );
        assert_eq!(
            expected_upstream_requests(false, true, false),
            EXPECTED_COOKING_REQUESTS - 1
        );
        assert_eq!(
            expected_upstream_requests(false, false, true),
            BASE_COOKING_REQUESTS
        );
        assert_eq!(
            expected_upstream_requests(false, false, false),
            BASE_COOKING_REQUESTS
        );
    }

    #[test]
    fn credential_class_diagnostic_distinguishes_rotation_without_exposing_values() {
        assert_eq!(credential_class(MODEL_SENTINEL), "model_initial");
        assert_eq!(credential_class(MODEL_ROTATED_SENTINEL), "model_rotated");
        assert_ne!(
            credential_class(MODEL_SENTINEL),
            credential_class(MODEL_ROTATED_SENTINEL)
        );
    }

    #[test]
    fn crash_mode_relay_ledger_tracks_all_five_logical_recipes() {
        assert_eq!(
            expected_relay_ledger(true, true),
            "['recipe-51','recipe-52','recipe-53','recipe-54','recipe-55']"
        );
        assert_eq!(
            expected_relay_ledger(false, false),
            "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46']"
        );
        assert_eq!(
            expected_relay_ledger(true, false),
            "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46','recipe-47','recipe-48','recipe-49']"
        );
    }
}
