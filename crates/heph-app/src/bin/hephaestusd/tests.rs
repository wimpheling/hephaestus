use super::{
    secrets::{append_repository_image_mount_roots, load_secret_keys},
    support::parse_ui_origin_config,
};
use oci_builder_runtime_local::LocalOciRuntimeConfig;
use secret_store::KeyProvider;
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf};

#[test]
fn loads_a_strict_multi_version_key_directory() {
    let temporary = tempfile::tempdir().expect("key directory parent");
    let directory = temporary.path().join("keys");
    fs::create_dir(&directory).expect("key directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("key directory mode");
    for (reference, byte) in [("local-v1", 1_u8), ("local-v2", 2_u8)] {
        let path = directory.join(reference);
        fs::write(&path, [byte; 32]).expect("key file");
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).expect("key file mode");
    }
    let provider = load_secret_keys(&directory, String::from("local-v2")).expect("valid key ring");
    assert_eq!(
        provider.active_key_reference().expect("active key"),
        "local-v2"
    );
    assert!(provider.key("local-v1").is_ok());

    fs::set_permissions(
        directory.join("local-v1"),
        fs::Permissions::from_mode(0o600),
    )
    .expect("unsafe key mode");
    assert!(load_secret_keys(&directory, String::from("local-v2")).is_err());
}

#[test]
fn repository_image_vm_roots_include_each_operation_input_and_output() {
    let runtime = LocalOciRuntimeConfig {
        repository_root: PathBuf::from("/repositories"),
        checkout_root: PathBuf::from("/private/checkouts"),
        image_layouts: BTreeMap::from([(
            String::from("registry.example/base@sha256:aaaaaaaa"),
            PathBuf::from("/private/bases/approved"),
        )]),
        output_root: PathBuf::from("/private/candidates"),
        verified_rootfs_root: Some(PathBuf::from("/private/verification")),
        git_binary: PathBuf::from("/usr/bin/git"),
        tar_binary: PathBuf::from("/usr/bin/tar"),
        buildah_binary: None,
        trivy_binary: None,
        umoci_binary: None,
        buildah_output_prefix: String::from("heph-builder"),
    };
    let mut roots = vec![PathBuf::from("/workspace")];

    append_repository_image_mount_roots(
        &mut roots,
        &runtime,
        &PathBuf::from("/private/verification"),
    );

    assert_eq!(
        roots,
        vec![
            PathBuf::from("/workspace"),
            PathBuf::from("/private/checkouts"),
            PathBuf::from("/private/candidates"),
            PathBuf::from("/private/verification"),
            PathBuf::from("/private/bases/approved"),
        ]
    );
}

#[test]
fn ui_origin_parser_is_opt_in_without_global_environment() {
    assert!(
        parse_ui_origin_config(false, None, None, None, None)
            .expect("disabled")
            .is_none()
    );
    assert!(
        parse_ui_origin_config(
            false,
            Some("127.0.0.1:19091"),
            Some("ui.example.test"),
            None,
            Some("https://example.test"),
        )
        .is_err()
    );
}

#[test]
fn ui_origin_parser_rejects_partial_and_non_loopback_configuration() {
    assert!(
        parse_ui_origin_config(
            true,
            Some("127.0.0.1:19091"),
            Some("ui.example.test"),
            None,
            None,
        )
        .is_err()
    );
    assert!(
        parse_ui_origin_config(
            true,
            Some("0.0.0.0:19091"),
            Some("ui.example.test"),
            None,
            Some("https://example.test"),
        )
        .is_err()
    );
}

#[test]
fn ui_origin_parser_accepts_independent_canonical_ports() {
    let config = parse_ui_origin_config(
        true,
        Some("127.0.0.1:19091"),
        Some("ui.example.test"),
        Some("9443"),
        Some("https://example.test:8443"),
    )
    .expect("valid UI origin")
    .expect("enabled UI origin");
    assert_eq!(config.public_port().get(), 9443);
    assert_eq!(config.platform_origin(), "https://example.test:8443");
    assert!(
        parse_ui_origin_config(
            true,
            Some("127.0.0.1:19091"),
            Some("ui.example.test"),
            Some("09443"),
            Some("https://example.test"),
        )
        .is_err()
    );
}
