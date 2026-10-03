use super::*;

fn scope(namespace: &str, host: &str) -> RunCleanupHostId {
    RunCleanupHostId::new(namespace.to_owned(), host.to_owned()).unwrap()
}

#[test]
fn provider_host_scope_is_checked_and_bounded() {
    let excessive = "x".repeat(129);
    for invalid in ["", "..", "a/b", "a\0b", "a\nb", "a b", &excessive] {
        assert_eq!(
            RunCleanupHostId::new(invalid.to_owned(), String::from("host-1")),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
        assert_eq!(
            RunCleanupHostId::new(String::from("local-provider-1"), invalid.to_owned()),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
    }
    assert_eq!(host().provider_namespace(), "local-provider-1");
    assert_eq!(host().host_id(), "host-1");
}

#[test]
fn another_host_or_provider_cannot_report_absence_or_destruction() {
    let exact = target(persisted(), 1, vec![]);
    for wrong in [
        scope("local-provider-1", "host-2"),
        scope("replacement-provider", "host-1"),
    ] {
        for observation in [
            RunCleanupVmObservation::destroyed(wrong.clone(), VmId(String::from("vm-1"))).unwrap(),
            RunCleanupVmObservation::authoritatively_absent(
                wrong.clone(),
                VmId(String::from("vm-1")),
            )
            .unwrap(),
        ] {
            assert_eq!(
                RunCleanupReceipt::new(exact.clone(), observation, OffsetDateTime::UNIX_EPOCH),
                Err(RunCleanupError::HostMismatch)
            );
        }
    }
}

#[test]
fn zero_volume_digest_still_binds_host_and_provider_owner() {
    let exact = target(persisted(), 1, vec![]);
    for changed in [
        scope("local-provider-1", "host-2"),
        scope("replacement-provider", "host-1"),
    ] {
        let other = target(
            RunCleanupVmTarget::persisted(changed, VmId(String::from("vm-1"))).unwrap(),
            1,
            vec![],
        );
        assert_ne!(exact.digest(), other.digest());
        assert_eq!(
            receipt(exact.clone()).matches_target(&other),
            Err(RunCleanupError::TargetMismatch)
        );
    }
}

#[test]
fn mixed_host_history_holds_fences_without_claiming_proven_vm_scope() {
    let leases = vec![lease(10, 20, "host-1", 7), lease(11, 21, "host-2", 8)];
    assert_eq!(
        RunCleanupTarget::new(
            id(1).parse().unwrap(),
            id(2).parse().unwrap(),
            id(3).parse().unwrap(),
            persisted(),
            1,
            leases.clone()
        ),
        Err(RunCleanupError::HostMismatch)
    );
    let unknown = target(
        RunCleanupVmTarget::historical_unresolved(),
        1,
        leases.clone(),
    );
    assert_eq!(unknown.leases(), leases.as_slice());
    assert!(unknown.vm_target().host().is_none());
    assert_eq!(
        RunCleanupReceipt::new(
            unknown,
            RunCleanupVmObservation::authoritatively_absent(host(), VmId(String::from("vm-1")))
                .unwrap(),
            OffsetDateTime::UNIX_EPOCH
        ),
        Err(RunCleanupError::UnresolvedVmIdentity)
    );
}
