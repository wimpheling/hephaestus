use super::support::VALID;
use crate::{PublicationMode, REUSABLE_RELEASE_VERSION, parse};
use forge_domain::GitRef;

#[test]
fn parses_and_matches_valid_release() {
    let parsed = parse(VALID.as_bytes());
    let config = parsed.config.expect("valid config");
    assert_eq!(config.version, REUSABLE_RELEASE_VERSION);
    assert_eq!(config.publication.mode, PublicationMode::Proposal);
    assert!(!config.publication.mode.permits_git_write_remote());
    assert!(parsed.diagnostics.is_empty());
    assert!(
        config
            .triggers
            .matches(&GitRef::parse("refs/heads/main").expect("valid ref"))
    );
    assert!(
        !config
            .triggers
            .matches(&GitRef::parse("refs/tags/v1").expect("valid ref"))
    );
    assert_eq!(parsed.hash.as_str().len(), 64);
    assert_eq!(
        parsed
            .normalized_hash
            .expect("valid config has normalized hash")
            .as_str()
            .len(),
        64
    );
}

#[test]
fn publication_mode_is_explicit_and_legacy_configs_default_to_proposal() {
    let legacy = parse(VALID.as_bytes());
    let legacy_hash = legacy.normalized_hash.clone();
    assert_eq!(
        legacy
            .config
            .expect("legacy configuration")
            .publication
            .mode,
        PublicationMode::Proposal
    );

    let explicit_proposal = VALID.replace(
        "[state_volume]",
        "[publication]\nmode = \"proposal\"\n\n[state_volume]",
    );
    assert_eq!(
        parse(explicit_proposal.as_bytes()).normalized_hash,
        legacy_hash,
        "implicit and explicit proposal mode must normalize identically"
    );

    let runtime_git = VALID.replace("mount = true", "mount = false").replace(
        "[state_volume]",
        "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
         [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
         resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
         optional_operations = [\"update_ref\"]\nrequired = true\n\n\
         [capability_slots.git]\nref_globs = [\"refs/heads/content\"]\n\
         changed_path_globs = [\"content/**\"]\n\
         transfer = { request_bytes = 1048576, pack_bytes = 8388608, object_count = 10000, ref_updates = 8 }\n\n\
         [state_volume]",
    );
    let parsed = parse(runtime_git.as_bytes());
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics
    );
    assert_eq!(
        parsed
            .config
            .expect("runtime Git configuration")
            .publication
            .mode,
        PublicationMode::RuntimeGit
    );
}

#[test]
fn publication_modes_cannot_cross_workspace_authority_boundaries() {
    let writable_proposal = VALID.replace("read_only = true", "read_only = false");
    let parsed = parse(writable_proposal.as_bytes());
    assert_eq!(
        parsed.diagnostics[0].code,
        "proposal_workspace_must_be_read_only"
    );

    let runtime_git = VALID.replace(
        "[state_volume]",
        "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
         [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
         resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
         optional_operations = [\"update_ref\"]\nrequired = true\n\n[state_volume]",
    );
    let parsed = parse(runtime_git.as_bytes());
    assert_eq!(
        parsed.diagnostics[0].code,
        "runtime_git_uses_capability_worktrees"
    );
}

#[test]
fn runtime_git_requires_an_explicit_repository_capability_slot() {
    let missing = VALID.replace("mount = true", "mount = false").replace(
        "[state_volume]",
        "[publication]\nmode = \"runtime_git\"\n\n[state_volume]",
    );
    assert_eq!(
        parse(missing.as_bytes()).diagnostics[0].code,
        "runtime_git_repository_slot_required"
    );

    let wrong_kind = VALID.replace("mount = true", "mount = false").replace(
        "[state_volume]",
        "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"state\"\n\n\
         [[capability_slots]]\nkey = \"state\"\npurpose = \"Use state\"\n\
         resource_kind = \"state_volume\"\nrequired_operations = [\"attach\"]\n\
         required = true\n\n[state_volume]",
    );
    assert_eq!(
        parse(wrong_kind.as_bytes()).diagnostics[0].code,
        "runtime_git_repository_slot_invalid"
    );

    let proposal_slot = VALID.replace(
        "[state_volume]",
        "[publication]\nmode = \"proposal\"\nrepository_slot = \"content\"\n\n\
         [state_volume]",
    );
    assert_eq!(
        parse(proposal_slot.as_bytes()).diagnostics[0].code,
        "proposal_repository_slot_forbidden"
    );
}

#[test]
fn reports_version_and_field_diagnostics() {
    let invalid = VALID.replace("version = 2", "version = 99");
    let parsed = parse(invalid.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].code, "unsupported_version");
}

#[test]
fn reports_syntax_errors_without_panicking() {
    let parsed = parse(b"version = [");
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_toml");
}

#[test]
fn rejects_unsafe_and_duplicate_declared_results() {
    let invalid = VALID.replace(
        r#"declared_files = ["reports/review.json"]"#,
        r#"declared_files = ["../escape", "report.json", "report.json"]"#,
    );
    let parsed = parse(invalid.as_bytes());
    assert!(parsed.config.is_none());
    assert_eq!(parsed.diagnostics[0].code, "invalid_declared_file");
    assert_eq!(parsed.diagnostics[1].code, "duplicate_declared_file");
}
