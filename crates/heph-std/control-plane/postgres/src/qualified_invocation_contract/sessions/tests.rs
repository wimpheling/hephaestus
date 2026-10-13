use super::{SessionObservation, validate};

fn observed(role: &str) -> SessionObservation {
    SessionObservation {
        pid: 42,
        datid: 43,
        database: "owned-fixture".into(),
        version: "17.9".into(),
        version_number: 170_009,
        login: "postgres".into(),
        current_role: role.into(),
        selected_role: if role == "postgres" { "none" } else { role }.into(),
        owner: "postgres".into(),
        superuser: role == "postgres",
        can_login: role == "postgres",
        bypass_rls: role != "hephaestus_app",
        isolation: "repeatable read".into(),
        read_only: "on".into(),
        search_path: "pg_catalog, public, pg_temp".into(),
        timezone: "UTC".into(),
        event_context: vec![None, None, None, None],
        nonce: "held-observation".into(),
    }
}

#[test]
fn supported_major_accepts_distinct_patch_versions_without_role_relaxation() {
    for version in [170_000, 170_001, 170_009, 170_010, 170_099] {
        for role in ["postgres", "hephaestus_worker", "hephaestus_app"] {
            let mut context = observed(role);
            context.version_number = version;
            let selected = if role == "postgres" { "none" } else { role };
            validate(&context, role, selected, "held-observation").unwrap();
        }
    }
}

#[test]
fn other_majors_and_invalid_numbers_are_denied_with_safe_version_diagnostic() {
    for version in [-170_009, 0, 99_999, 160_010, 180_000, 190_009] {
        let mut context = observed("hephaestus_app");
        context.version_number = version;
        let error = validate(
            &context,
            "hephaestus_app",
            "hephaestus_app",
            "held-observation",
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires PostgreSQL major 17"));
        assert!(error.to_string().contains(&version.to_string()));
    }
}

#[test]
fn supported_version_keeps_all_original_role_owner_and_transaction_denials() {
    type Change = fn(&mut SessionObservation);
    let changes: &[Change] = &[
        |c| c.pid = 0,
        |c| c.datid = 0,
        |c| c.login = "borrowed-login".into(),
        |c| c.current_role = "postgres".into(),
        |c| c.selected_role = "none".into(),
        |c| c.owner = "foreign-owner".into(),
        |c| c.superuser = true,
        |c| c.can_login = true,
        |c| c.bypass_rls = true,
        |c| c.isolation = "read committed".into(),
        |c| c.read_only = "off".into(),
        |c| c.search_path = "public".into(),
        |c| c.timezone = "Europe/Lisbon".into(),
        |c| c.event_context[0] = Some("saved-actor".into()),
        |c| c.event_context[1] = Some("saved-request".into()),
        |c| c.event_context[2] = Some("saved-subject".into()),
        |c| c.event_context[3] = Some("saved-occurrence".into()),
        |c| c.nonce = "another-backend".into(),
    ];
    for change in changes {
        let mut context = observed("hephaestus_app");
        change(&mut context);
        assert!(
            validate(
                &context,
                "hephaestus_app",
                "hephaestus_app",
                "held-observation"
            )
            .is_err()
        );
    }
    let mut worker = observed("hephaestus_worker");
    worker.bypass_rls = false;
    assert!(
        validate(
            &worker,
            "hephaestus_worker",
            "hephaestus_worker",
            "held-observation"
        )
        .is_err()
    );
    let mut admin = observed("postgres");
    admin.superuser = false;
    assert!(validate(&admin, "postgres", "none", "held-observation").is_err());
}
