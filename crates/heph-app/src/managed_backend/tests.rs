//! Pure denial/no-write checks; no fabricated provider or physical positive proof.

use super::{ManagedRootIntent, custody, history, paths, roots, validation};
use run_domain::{VmOwnerHistory, VmOwnerHistoryConflict, VmOwnerHistoryUnknown};
use std::{fs, os::unix::fs::symlink, path::Path};
use volume_trait::{OwnedVolumeRootHistory, OwnedVolumeRootHistoryConflict, VolumeRootNamespaceId};

#[test]
fn only_explicit_bootstrap_with_observed_no_history_passes_comparison() {
    assert!(
        history::check_vm(
            ManagedRootIntent::BootstrapEmpty,
            VmOwnerHistory::NoHistoryForConfiguredHost
        )
        .is_ok()
    );
    assert!(history::check_vm(ManagedRootIntent::BootstrapEmpty, VmOwnerHistory::Exact).is_err());
    assert!(history::check_vm(ManagedRootIntent::OpenExisting, VmOwnerHistory::Exact).is_ok());
    assert!(
        history::check_vm(
            ManagedRootIntent::OpenExisting,
            VmOwnerHistory::NoHistoryForConfiguredHost
        )
        .is_ok()
    );
}

#[test]
fn unknown_global_or_foreign_namespace_never_becomes_filtered_absence() {
    for history in [
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch),
        VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::HistoricalUnproven),
        VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::MissingOwnerIdentity),
        VmOwnerHistory::Unknown(VmOwnerHistoryUnknown::UnqualifiedOriginal),
    ] {
        for intent in [
            ManagedRootIntent::BootstrapEmpty,
            ManagedRootIntent::OpenExisting,
        ] {
            assert!(history::check_vm(intent, history).is_err());
        }
    }
}

#[test]
fn volume_history_requires_exact_namespace_without_bootstrap_adoption() {
    let namespace = VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(1)).unwrap();
    let other = VolumeRootNamespaceId::from_uuid(uuid::Uuid::from_u128(2)).unwrap();
    assert!(
        history::check_volume(
            ManagedRootIntent::BootstrapEmpty,
            namespace,
            OwnedVolumeRootHistory::NoOwnedHistory
        )
        .is_ok()
    );
    assert!(
        history::check_volume(
            ManagedRootIntent::OpenExisting,
            namespace,
            OwnedVolumeRootHistory::Existing { namespace }
        )
        .is_ok()
    );
    for intent in [
        ManagedRootIntent::BootstrapEmpty,
        ManagedRootIntent::OpenExisting,
    ] {
        assert!(
            history::check_volume(
                intent,
                namespace,
                OwnedVolumeRootHistory::Existing { namespace: other }
            )
            .is_err()
        );
        assert!(
            history::check_volume(
                intent,
                namespace,
                OwnedVolumeRootHistory::Conflict {
                    code: OwnedVolumeRootHistoryConflict::RootRedirect
                }
            )
            .is_err()
        );
        assert!(
            history::check_volume(
                intent,
                namespace,
                OwnedVolumeRootHistory::Conflict {
                    code: OwnedVolumeRootHistoryConflict::MultipleOwnerIdentities
                }
            )
            .is_err()
        );
    }
    assert!(
        history::check_volume(
            ManagedRootIntent::BootstrapEmpty,
            namespace,
            OwnedVolumeRootHistory::Existing { namespace }
        )
        .is_err()
    );
}

#[test]
fn durable_vm_namespace_is_explicit_canonical_and_nonnil() {
    for invalid in [
        "",
        "owner",
        "00000000-0000-0000-0000-000000000000",
        "00112233445566778899aabbccddeeff",
        "00112233-4455-6677-8899-AABBCCDDEEFF",
    ] {
        assert!(validation::namespace(invalid).is_err());
    }
    assert!(validation::namespace("00112233-4455-6677-8899-aabbccddeeff").is_ok());
}

#[test]
fn dedicated_roots_reject_equal_and_nested_paths_but_not_prefix_siblings() {
    for (first, second) in [
        ("/run/managed", "/run/managed"),
        ("/run/managed", "/run/managed/ordinary"),
        ("/run/managed/child", "/run/managed"),
    ] {
        assert!(roots::disjoint(Path::new(first), Path::new(second)).is_err());
    }
    assert!(roots::disjoint(Path::new("/run/managed"), Path::new("/run/managed-other")).is_ok());
}

#[test]
fn readonly_path_validation_creates_no_missing_root_and_rejects_aliases() {
    let parent = tempfile::tempdir().unwrap();
    let absent = parent.path().join("managed");
    paths::directory(&absent, true, true).unwrap();
    assert!(!absent.exists());
    let alias = parent.path().join("alias");
    symlink(parent.path(), &alias).unwrap();
    assert!(paths::directory(&alias.join("managed"), true, true).is_err());
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 1);
}

#[test]
fn assembled_managed_state_does_not_open_worker_readiness() {
    assert!(custody::pending_readiness(false).is_err());
    assert!(custody::pending_readiness(true).is_err());
}

#[test]
fn acquisition_errors_retain_actual_source_instead_of_component_text() {
    use std::error::Error;
    let error = custody::failure(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .is_some()
    );
}

#[test]
fn custom_ordinary_provider_cannot_hide_mutable_roots() {
    use crate::VmBackendConfig;
    use std::sync::Arc;
    let provider = Arc::new(vm_fake::FakeProvider::new());
    assert!(validation::inspectable(&VmBackendConfig::Custom(provider.clone())).is_err());
    assert!(
        validation::inspectable(&VmBackendConfig::CustomWithRuntimeGitSocket {
            provider,
            runtime_git_socket_path: "/run/runtime-git.sock".into(),
        })
        .is_err()
    );
    assert!(validation::inspectable(&VmBackendConfig::Fake).is_ok());
    assert!(validation::inspectable(&VmBackendConfig::FixtureResult).is_ok());
}
