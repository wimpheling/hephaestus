// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
#[cfg(test)]
mod tests {
    use super::{UpdateVariant, apply_update_variant, copy_source_tree, redact_build_log};
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    #[test]
    fn source_copy_excludes_generated_and_git_directories() {
        let root = std::env::temp_dir().join(format!("cooking-build-helper-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(source.join("vendor")).expect("vendor directory");
        fs::create_dir_all(source.join(".cargo")).expect("cargo directory");
        fs::create_dir_all(source.join("target")).expect("target directory");
        fs::create_dir_all(source.join("__pycache__")).expect("Python cache directory");
        fs::create_dir_all(source.join("nested/.git")).expect("nested git directory");
        fs::write(source.join("agent.toml"), b"agent").expect("agent config");
        fs::write(source.join("vendor/config"), b"vendored").expect("vendor file");
        fs::write(source.join(".cargo/config.toml"), b"cargo").expect("cargo file");
        fs::write(source.join("target/stale"), b"stale").expect("target file");
        fs::write(source.join("__pycache__/cooking.cpython-314.pyc"), b"stale")
            .expect("Python cache file");
        fs::write(source.join("generated.pyc"), b"stale").expect("Python bytecode file");
        fs::write(source.join("nested/.git/stale"), b"stale").expect("git file");
        fs::set_permissions(source.join("agent.toml"), fs::Permissions::from_mode(0o640))
            .expect("source mode");

        copy_source_tree(&source, &destination).expect("copy source");
        assert_eq!(
            fs::read(destination.join("agent.toml")).expect("agent"),
            b"agent"
        );
        assert_eq!(
            fs::read(destination.join("vendor/config")).expect("vendor"),
            b"vendored"
        );
        assert_eq!(
            fs::read(destination.join(".cargo/config.toml")).expect("cargo"),
            b"cargo"
        );
        assert!(!destination.join("target").exists());
        assert!(!destination.join("__pycache__").exists());
        assert!(!destination.join("generated.pyc").exists());
        assert!(!destination.join("nested/.git").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canonical_cooking_blog_dockerfile_satisfies_production_policy() {
        let dockerfile = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cooking/cooking-blog/Dockerfile");
        let source = fs::read_to_string(dockerfile).expect("canonical cooking blog Dockerfile");
        oci_builder_worker::DockerfilePolicy::validate(&source)
            .expect("canonical cooking blog Dockerfile policy");
    }

    #[test]
    fn build_log_redaction_covers_all_fixture_credentials() {
        let raw = concat!(
            "golden-brokered-provider-sentinel-5d1a ",
            "cooking-inbound-only-fixture-sentinel ",
            "cooking-model-only-fixture-sentinel-724c ",
            "cooking-relay-only-fixture-sentinel-819e ",
            "cooking-model-rotated-fixture-sentinel-936f ",
            "cooking-inbound-rotated-fixture-sentinel-157a ",
            "cooking-relay-rotated-fixture-sentinel-482b ",
            "telegram-bot-api-secret-token"
        );
        let redacted = redact_build_log(raw);
        assert!(!redacted.contains("sentinel"));
        assert!(!redacted.contains("telegram-bot-api-secret-token"));
        assert_eq!(redacted.matches("[REDACTED]").count(), 8);
    }

    #[test]
    fn update_variants_are_explicit_and_do_not_mutate_canonical_source() {
        let root = std::env::temp_dir().join(format!("cooking-update-variant-{}", Uuid::new_v4()));
        let canonical = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cooking/cooking-agent");
        let rollback = root.join("rollback");
        let abnormal = root.join("abnormal");
        let sequential = root.join("sequential");
        fs::create_dir_all(&root).expect("variant root");
        copy_source_tree(&canonical, &rollback).expect("rollback source");
        copy_source_tree(&canonical, &abnormal).expect("abnormal source");
        copy_source_tree(&canonical, &sequential).expect("sequential source");
        apply_update_variant(&rollback, UpdateVariant::Rollback).expect("rollback variant");
        apply_update_variant(&abnormal, UpdateVariant::Abnormal).expect("abnormal variant");
        apply_update_variant(&sequential, UpdateVariant::Rollback).expect("sequential rollback");
        apply_update_variant(&sequential, UpdateVariant::Abnormal).expect("sequential abnormal");

        let canonical_config =
            fs::read_to_string(canonical.join("agent.toml")).expect("canonical config");
        let rollback_config =
            fs::read_to_string(rollback.join("agent.toml")).expect("rollback config");
        let abnormal_config =
            fs::read_to_string(abnormal.join("agent.toml")).expect("abnormal config");
        let sequential_config =
            fs::read_to_string(sequential.join("agent.toml")).expect("sequential config");
        assert!(canonical_config.contains("arguments = [\"--migrate\"]"));
        assert!(rollback_config.contains("arguments = [\"--rollback-fixture\"]"));
        assert!(abnormal_config.contains("arguments = [\"--abnormal-fixture\"]"));
        assert!(sequential_config.contains("arguments = [\"--abnormal-fixture\"]"));
        assert!(
            fs::read_to_string(rollback.join("cooking_agent.py"))
                .expect("rollback source")
                .contains("deliberate migration rollback from v2")
        );
        let abnormal_source =
            fs::read_to_string(abnormal.join("cooking_agent.py")).expect("abnormal source");
        assert!(abnormal_source.contains("signal.SIGKILL"));
        let sequential_source =
            fs::read_to_string(sequential.join("cooking_agent.py")).expect("sequential source");
        assert!(sequential_source.contains("signal.SIGKILL"));
        assert_eq!(
            fs::read_to_string(canonical.join("agent.toml")).expect("canonical remains intact"),
            canonical_config
        );
        let _ = fs::remove_dir_all(root);
    }
}
