use super::*;

#[test]
fn gateway_invalidation_is_transactional_and_uses_the_durable_product_outbox() {
    assert!(GATEWAY_EVENTS_MIGRATION.contains("capture_gateway_application_event"));
    assert!(GATEWAY_EVENTS_MIGRATION.contains("gateway.changed"));
    assert!(
        GATEWAY_EVENTS_MIGRATION
            .contains("AFTER INSERT OR UPDATE OF lifecycle, active_revision_id")
    );
    assert!(!GATEWAY_EVENTS_MIGRATION.contains("request bodies"));
    assert!(!GATEWAY_EVENTS_MIGRATION.contains("secret values"));
}

#[test]
fn every_authoritative_product_state_path_has_a_capture_trigger() {
    let sources = [
        "users",
        "external_identities",
        "user_profiles",
        "organizations",
        "organization_members",
        "organization_secret_managers",
        "projects",
        "project_maintainers",
        "project_secret_roles",
        "repositories",
        "repository_managers",
        "repository_secret_roles",
        "agent_families",
        "git_receives",
        "git_refs",
        "git_ref_updates",
        "agent_config_revisions",
        "build_requests",
        "build_request_sources",
        "build_executions",
        "releases",
        "release_agents",
        "release_artifacts",
        "agent_instances",
        "agent_instance_revisions",
        "agent_attachments",
        "agent_updates",
        "agent_instance_state_volumes",
        "deferred_agent_triggers",
        "agent_instance_events",
        "agent_instance_volume_leases",
        "runs",
        "run_requests",
        "run_results",
        "run_workspaces",
        "run_instance_provenance",
        "run_secret_provenance",
        "result_artifacts",
        "review_proposals",
        "control_requests",
        "secrets",
        "secret_versions",
        "secret_grants",
        "secret_imports",
        "agent_secret_bindings",
        "secret_leases",
        "secret_runtime_sessions",
        "secret_runtime_mounts",
    ];
    for table in sources {
        assert!(
            MIGRATION.contains(&format!(" ON {table}\n")),
            "missing application-event trigger for {table}"
        );
    }
}

#[test]
fn product_outbox_is_trigger_only_and_dead_letters_do_not_pin_retention() {
    assert!(MIGRATION.contains("AFTER INSERT ON application_events"));
    assert!(MIGRATION.contains("INSERT INTO product_event_outbox (event_id) VALUES (NEW.id)"));
    assert!(!MIGRATION.contains("GRANT INSERT ON product_event_outbox"));
    assert!(MIGRATION.contains("AND outbox.dead_lettered_at IS NULL"));
    assert!(MIGRATION.contains("CREATE TABLE product_event_dead_letters"));
    assert!(MIGRATION.contains("CHECK (num_nonnulls(published_at, dead_lettered_at) <= 1)"));
}

#[test]
fn persisted_projection_domain_is_finite_and_non_disclosing() {
    for aggregate in [
        "identity_profile",
        "identity_organizations",
        "organization",
        "project",
        "repository",
        "repository_ref",
        "build",
        "release",
        "agent_instance",
        "run",
        "review",
        "secret_metadata",
        "secret_grant",
        "secret_import",
        "agent_secret_binding",
        "artifact",
    ] {
        assert!(MIGRATION.contains(&format!("'{aggregate}'")));
    }
    assert!(MIGRATION.contains("unknown application event state %"));
    assert!(MIGRATION.contains("CASE aggregate_type"));
    assert!(!MIGRATION.contains("safe_ref text"));
    assert!(!MIGRATION.contains("old_commit text"));
    assert!(!MIGRATION.contains("new_commit text"));
}

#[test]
fn secret_authority_changes_share_occurrence_across_owner_and_project() {
    let parented = MIGRATION
        .split("CREATE FUNCTION capture_parented_application_event")
        .nth(1)
        .expect("parented capture function");
    for table in ["secret_grants", "secret_imports"] {
        let route = parented
            .split(&format!("WHEN '{table}' THEN"))
            .nth(1)
            .expect("secret route");
        let route = route.split("WHEN '").next().expect("bounded route");
        assert!(route.contains("v_occurrence, 'organization'"));
        assert!(route.contains("v_occurrence, 'project'"));
        assert!(!route.contains("v_occurrence, 'repository'"));
    }
}

#[test]
fn run_changes_share_one_occurrence_across_run_instance_and_project_scopes() {
    let parented = MIGRATION
        .split("CREATE FUNCTION capture_parented_application_event")
        .nth(1)
        .expect("parented capture function");
    let route = parented
        .split("WHEN 'runs' THEN")
        .nth(1)
        .expect("run route")
        .split("WHEN 'result_artifacts' THEN")
        .next()
        .expect("bounded run route");

    assert_eq!(route.matches("v_occurrence").count(), 3);
    assert!(route.contains("v_occurrence, 'run', v_id, 'run', v_id"));
    assert!(route.contains("v_occurrence, 'agent_instance', v_parent, 'run', v_id"));
    assert!(route.contains("v_occurrence, 'project', v_project, 'run', v_id"));
    assert!(MIGRATION.contains("CREATE TRIGGER runs_application_event"));
    assert!(!MIGRATION.contains("CREATE TRIGGER runs_run_application_event"));
}

#[test]
fn capture_uses_stable_transaction_occurrence_for_idempotent_receipts() {
    let occurrence_expression =
        "NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid";
    assert_eq!(MIGRATION.matches(occurrence_expression).count(), 2);
    assert!(!MIGRATION.contains("gen_random_uuid(), TG_ARGV[2]"));
}

#[test]
fn internal_outbox_cannot_be_misclassified_as_product_events() {
    assert!(MIGRATION.contains("'internal_command'"));
    assert!(MIGRATION.contains("'internal_signal'"));
    assert!(!MIGRATION.contains("message_class IN ('product_event'"));
    assert!(MIGRATION.contains("CHECK (subject = 'hephaestus.product.event.v1')"));
    let retirement = MIGRATION
        .split("UPDATE outbox")
        .nth(1)
        .expect("bounded legacy retirement")
        .split(");")
        .next()
        .expect("retirement statement");
    assert!(retirement.contains("WHERE published_at IS NULL AND subject IN ("));
    assert!(retirement.contains("'hephaestus.release.published.v1'"));
    assert!(retirement.contains("'hephaestus.git.receive.accepted'"));
    assert!(retirement.contains("'hephaestus.git.agent_config.invalid'"));
    assert!(retirement.contains("'heph.run.event.lifecycle.v1'"));
    assert!(retirement.contains("'hephaestus.secret.reconcile_revocation.v1'"));
    for actionable in [
        "hephaestus.build.requested.v1",
        "hephaestus.instance.run.requested.v1",
        "heph.run.command.start.v1",
        "heph.run.command.cancel.v1",
        "hephaestus.control.execute",
    ] {
        assert!(MIGRATION.contains(&format!("'{actionable}'")));
        assert!(!retirement.contains(actionable));
    }
}
