use super::support::VALID;
use crate::parse;

#[test]
fn parses_symbolic_capability_declarations() {
    let source = format!(
        r#"{VALID}

[[capability_slots]]
key = "source"
purpose = "Read source and optionally trigger its configured release"
resource_kind = "repository"
required_operations = ["git_read", "inspect"]
optional_operations = ["trigger_run"]
required = true
"#
    );
    let parsed = parse(source.as_bytes());
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics
    );
    let config = parsed.config.expect("valid capability declaration");
    let declaration = &config.capability_slots[0];
    assert_eq!(declaration.key, "source");
    assert!(declaration.required);
    assert_eq!(declaration.required_operations.len(), 2);
    assert_eq!(declaration.optional_operations.len(), 1);
}

#[test]
fn capability_hash_normalizes_slot_and_operation_order() {
    let left = format!(
        r#"{VALID}

[[capability_slots]]
key = "source"
purpose = "Source repository"
resource_kind = "repository"
required_operations = ["git_read", "inspect"]
optional_operations = ["trigger_run"]
required = true

[[capability_slots]]
key = "worker"
purpose = "Worker instance"
resource_kind = "agent_instance"
required_operations = ["execute", "inspect"]
optional_operations = ["recover", "pause"]
"#
    );
    let right = format!(
        r#"{VALID}

[[capability_slots]]
key = "worker"
purpose = "Worker instance"
resource_kind = "agent_instance"
required_operations = ["inspect", "execute"]
optional_operations = ["pause", "recover"]

[[capability_slots]]
key = "source"
purpose = "Source repository"
resource_kind = "repository"
required_operations = ["inspect", "git_read"]
optional_operations = ["trigger_run"]
required = true
"#
    );

    let left = parse(left.as_bytes());
    let right = parse(right.as_bytes());
    assert!(left.diagnostics.is_empty(), "{:?}", left.diagnostics);
    assert!(right.diagnostics.is_empty(), "{:?}", right.diagnostics);
    assert_eq!(left.normalized_hash, right.normalized_hash);
    assert_ne!(left.hash, right.hash);
}

#[test]
fn rejects_duplicate_slots_and_invalid_operation_sets() {
    let cases = [
        (
            r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect"]
[[capability_slots]]
key = "source"
purpose = "Duplicate"
resource_kind = "repository"
required_operations = ["git_read"]
"#,
            "duplicate_capability_slot",
        ),
        (
            r#"
[[capability_slots]]
key = "entrypoint"
purpose = "Gateway"
resource_kind = "gateway"
required_operations = ["git_read"]
"#,
            "illegal_capability_operation",
        ),
        (
            r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect", "inspect"]
"#,
            "duplicate_capability_operation",
        ),
        (
            r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
required_operations = ["inspect"]
optional_operations = ["inspect"]
"#,
            "overlapping_capability_operations",
        ),
        (
            r#"
[[capability_slots]]
key = "source"
purpose = "Source"
resource_kind = "repository"
"#,
            "empty_capability_operations",
        ),
    ];

    for (declaration, expected_code) in cases {
        let parsed = parse(format!("{VALID}\n{declaration}").as_bytes());
        assert!(parsed.config.is_none(), "case {expected_code} was accepted");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected_code),
            "missing {expected_code}: {:?}",
            parsed.diagnostics
        );
    }
}

#[test]
fn rejects_unknown_operations_and_tenant_binding_material() {
    let malformed_operation = format!(
        r#"{VALID}
[[capability_slots]]
key = "project"
purpose = "Project"
resource_kind = "project"
required_operations = ["delete_project"]
"#
    );
    let parsed = parse(malformed_operation.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");

    for forbidden in [
        r#"resource_id = "f774d581-c89e-4420-9712-24cc642d2a9a""#,
        r#"resource_name = "production""#,
        r#"granted_operations = ["inspect"]"#,
        r#"bearer_token = "must-never-be-accepted""#,
    ] {
        let source = format!(
            r#"{VALID}
[[capability_slots]]
key = "project"
purpose = "Project"
resource_kind = "project"
required_operations = ["inspect"]
{forbidden}
"#
        );
        let parsed = parse(source.as_bytes());
        assert!(parsed.config.is_none(), "forbidden field was accepted");
        assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
        assert!(
            !format!("{:?}", parsed.diagnostics).contains("must-never-be-accepted"),
            "parser diagnostics must not echo rejected bearer material"
        );
    }
}
