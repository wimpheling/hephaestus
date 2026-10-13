//! Pinned-root ordering with controlled files; no native cgroup proof.
use super::{PinnedCleanupRoots, ProviderOwner, tests::config};
use std::fs;
use tempfile::TempDir;
use vm_trait::VmId;

#[test]
fn uncertain_cgroup_preserves_original_pinned_runtime_and_foreign_replacement() {
    let temp = TempDir::new().unwrap();
    let mut config = config(&temp);
    let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
    let guard = owner.validate(&config).unwrap();
    let roots = PinnedCleanupRoots::from_guard(&guard).unwrap();
    let id = VmId("vm".into());
    let runtime = config.runtime_root.join(&id.0);
    let cgroup = config.cgroup_root.join(&id.0);
    fs::create_dir(&runtime).unwrap();
    fs::write(runtime.join("sentinel"), b"original bytes").unwrap();
    fs::create_dir(&cgroup).unwrap();
    let original = temp.path().join("original-runtime");
    fs::rename(&config.runtime_root, &original).unwrap();
    fs::create_dir(&config.runtime_root).unwrap();
    fs::create_dir(config.runtime_root.join(&id.0)).unwrap();
    let foreign = config.runtime_root.join(&id.0).join("sentinel");
    fs::write(&foreign, b"foreign bytes").unwrap();
    config.enforce_cgroup_v2 = true;
    assert!(roots.cleanup(&config, &id).is_err());
    assert_eq!(
        fs::read(original.join(&id.0).join("sentinel")).unwrap(),
        b"original bytes"
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign bytes");
    fs::write(cgroup.join("cgroup.events"), "populated 0\n").unwrap();
    // Regular fixture files prevent rmdir even after a valid empty observation.
    // Failed physical removal must still preserve the owned runtime.
    assert!(roots.cleanup(&config, &id).is_err());
    assert!(original.join(&id.0).is_dir());
    fs::remove_file(cgroup.join("cgroup.events")).unwrap();
    fs::remove_dir(&cgroup).unwrap();
    // Simulate completed external removal of this exact controlled child. This
    // tests the existing known-absence retry contract, not native emptiness.
    roots.cleanup(&config, &id).unwrap();
    roots.cleanup(&config, &id).unwrap();
    assert!(!original.join(&id.0).exists());
    assert_eq!(fs::read(foreign).unwrap(), b"foreign bytes");
}
