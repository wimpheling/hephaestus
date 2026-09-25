use super::support::Fixture;
use crate::validation::validate_config;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use vm_trait::VmError;

#[test]
fn host_preflight_returns_typed_configuration_errors() {
    let fixture = Fixture::new();
    let mut root_service = fixture.valid_config();
    root_service.service_uid = 0;
    assert!(matches!(
        validate_config(&root_service),
        Err(VmError::InvalidSpec { field, .. }) if field == "service_uid"
    ));

    let mut missing_kvm = fixture.valid_config();
    missing_kvm.kvm_device = fixture.temp.path().join("missing-kvm");
    assert!(matches!(
        validate_config(&missing_kvm),
        Err(VmError::Unavailable { resource, .. }) if resource == "KVM"
    ));

    let non_executable = fixture.temp.path().join("not-executable");
    fs::write(&non_executable, []).unwrap();
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();
    let mut invalid_passt = fixture.valid_config();
    invalid_passt.passt_binary = non_executable;
    assert!(matches!(
        validate_config(&invalid_passt),
        Err(VmError::InvalidSpec { field, .. }) if field == "passt_binary"
    ));

    let mut missing_delegation = fixture.valid_config();
    missing_delegation.enforce_cgroup_v2 = true;
    assert!(matches!(
        validate_config(&missing_delegation),
        Err(VmError::InvalidSpec { field, .. }) if field == "cgroup_root"
    ));
}

#[test]
fn host_preflight_rejects_relative_roots_and_zero_limits() {
    let fixture = Fixture::new();
    let mut relative = fixture.valid_config();
    relative.image_roots = vec![PathBuf::from("images")];
    assert!(matches!(
        validate_config(&relative),
        Err(VmError::InvalidSpec { field, .. }) if field == "image_roots"
    ));

    let mut zero_period = fixture.valid_config();
    zero_period.limits.cpu_period_micros = 0;
    assert!(matches!(
        validate_config(&zero_period),
        Err(VmError::InvalidSpec { field, .. }) if field == "limits.cpu_period_micros"
    ));

    let mut zero_memory = fixture.valid_config();
    zero_memory.limits.memory_max_bytes = 0;
    assert!(matches!(
        validate_config(&zero_memory),
        Err(VmError::InvalidSpec { field, .. }) if field == "limits.memory_max_bytes"
    ));

    let mut zero_pids = fixture.valid_config();
    zero_pids.limits.pids_max = 0;
    assert!(matches!(
        validate_config(&zero_pids),
        Err(VmError::InvalidSpec { field, .. }) if field == "limits.pids_max"
    ));
}
