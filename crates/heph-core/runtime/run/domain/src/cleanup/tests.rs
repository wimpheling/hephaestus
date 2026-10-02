use super::*;
use time::OffsetDateTime;
use vm_trait::VmId;

fn id(number: u32) -> String {
    format!("{number:032x}")
}

fn lease(volume: u32, identity: u32, host: &str, fence: i64) -> RunCleanupLeaseFence {
    RunCleanupLeaseFence::new(
        id(1).parse().unwrap(),
        id(identity).parse().unwrap(),
        id(volume).parse().unwrap(),
        host.to_owned(),
        fence,
    )
    .unwrap()
}

fn host() -> RunCleanupHostId {
    RunCleanupHostId::new(String::from("local-provider-1"), String::from("host-1")).unwrap()
}

fn persisted() -> RunCleanupVmTarget {
    RunCleanupVmTarget::persisted(host(), VmId(String::from("vm-1"))).unwrap()
}

fn target(
    vm: RunCleanupVmTarget,
    generation: u64,
    leases: Vec<RunCleanupLeaseFence>,
) -> RunCleanupTarget {
    RunCleanupTarget::new(
        id(1).parse().unwrap(),
        id(2).parse().unwrap(),
        id(3).parse().unwrap(),
        vm,
        generation,
        leases,
    )
    .unwrap()
}

fn receipt(target: RunCleanupTarget) -> RunCleanupReceipt {
    RunCleanupReceipt::new(
        target,
        RunCleanupVmObservation::destroyed(host(), VmId(String::from("vm-1"))).unwrap(),
        OffsetDateTime::UNIX_EPOCH,
    )
    .unwrap()
}

#[test]
fn complete_set_order_is_canonical() {
    let first = lease(10, 20, "host-1", 7);
    let second = lease(11, 21, "host-1", 8);
    let forward = target(persisted(), 1, vec![first.clone(), second.clone()]);
    let reversed = target(persisted(), 1, vec![second, first]);
    assert_eq!(forward, reversed);
    assert_eq!(forward.leases()[0].volume_id(), id(10).parse().unwrap());
    assert_eq!(forward.leases()[1].host_id(), "host-1");
}

#[test]
fn empty_set_still_requires_exact_vm_observation() {
    let empty = target(persisted(), 1, vec![]);
    let observation =
        RunCleanupVmObservation::authoritatively_absent(host(), VmId(String::from("vm-1")))
            .unwrap();
    let receipt =
        RunCleanupReceipt::new(empty.clone(), observation, OffsetDateTime::UNIX_EPOCH).unwrap();
    assert!(receipt.target().leases().is_empty());
    assert_eq!(
        receipt.observation().kind(),
        RunCleanupVmObservationKind::AuthoritativelyAbsent
    );
    assert!(receipt.matches_target(&empty).is_ok());
    let unresolved = target(RunCleanupVmTarget::historical_unresolved(), 1, vec![]);
    assert_eq!(
        RunCleanupReceipt::new(
            unresolved,
            receipt.observation().clone(),
            OffsetDateTime::UNIX_EPOCH
        ),
        Err(RunCleanupError::UnresolvedVmIdentity)
    );
}

#[test]
fn unresolved_history_preserves_the_fence_set() {
    let held = lease(10, 20, "host-1", 7);
    let unknown = target(
        RunCleanupVmTarget::historical_unresolved(),
        1,
        vec![held.clone()],
    );
    assert_eq!(unknown.leases(), &[held]);
    assert!(unknown.vm_target().vm_id().is_none());
    assert_ne!(
        unknown.digest(),
        target(persisted(), 1, unknown.leases().to_vec()).digest()
    );
    assert_eq!(
        RunCleanupReceipt::new(
            unknown,
            RunCleanupVmObservation::destroyed(host(), VmId(id(1))).unwrap(),
            OffsetDateTime::UNIX_EPOCH
        ),
        Err(RunCleanupError::UnresolvedVmIdentity)
    );
}

#[test]
fn invalid_generation_and_nil_outer_identities_reject() {
    for generation in [0, u64::MAX, u64::try_from(i64::MAX).unwrap() + 1] {
        assert_eq!(
            RunCleanupTarget::new(
                id(1).parse().unwrap(),
                id(2).parse().unwrap(),
                id(3).parse().unwrap(),
                persisted(),
                generation,
                vec![]
            ),
            Err(RunCleanupError::InvalidGeneration)
        );
    }
    for (run, instance, revision) in [(0, 2, 3), (1, 0, 3), (1, 2, 0)] {
        assert_eq!(
            RunCleanupTarget::new(
                id(run).parse().unwrap(),
                id(instance).parse().unwrap(),
                id(revision).parse().unwrap(),
                persisted(),
                1,
                vec![]
            ),
            Err(RunCleanupError::InvalidIdentity)
        );
    }
}

#[test]
fn lease_identity_and_fence_validation_rejects_missing_evidence() {
    for (run, identity, volume) in [(0, 20, 10), (1, 0, 10), (1, 20, 0)] {
        assert_eq!(
            RunCleanupLeaseFence::new(
                id(run).parse().unwrap(),
                id(identity).parse().unwrap(),
                id(volume).parse().unwrap(),
                String::from("host-1"),
                7
            ),
            Err(RunCleanupError::InvalidIdentity)
        );
    }
    for fence in [-1, 0] {
        assert_eq!(
            RunCleanupLeaseFence::new(
                id(1).parse().unwrap(),
                id(20).parse().unwrap(),
                id(10).parse().unwrap(),
                String::from("host-1"),
                fence
            ),
            Err(RunCleanupError::InvalidLeaseFence)
        );
    }
    assert_eq!(lease(10, 20, "host-1", i64::MAX).fencing_token(), i64::MAX);
}

#[test]
fn unsafe_host_and_vm_identities_reject() {
    let excessive = "a".repeat(129);
    for name in [
        "", ".", "..", "../vm", "vm/path", "a\0b", "a\nb", "a b", "é", &excessive,
    ] {
        assert_eq!(
            RunCleanupVmTarget::persisted(host(), VmId(name.to_owned())),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
        assert_eq!(
            RunCleanupVmTarget::planned(host(), VmId(name.to_owned())),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
        assert_eq!(
            RunCleanupVmObservation::destroyed(host(), VmId(name.to_owned())),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
        assert_eq!(
            RunCleanupLeaseFence::new(
                id(1).parse().unwrap(),
                id(20).parse().unwrap(),
                id(10).parse().unwrap(),
                name.to_owned(),
                7
            ),
            Err(RunCleanupError::InvalidProviderIdentity)
        );
    }
    assert!(RunCleanupVmTarget::persisted(host(), VmId("a".repeat(128))).is_ok());
}

#[test]
fn duplicate_lease_and_resource_identity_reject() {
    for (other, expected) in [
        (lease(11, 20, "host-1", 8), RunCleanupError::DuplicateLease),
        (lease(10, 21, "host-1", 8), RunCleanupError::DuplicateVolume),
    ] {
        assert_eq!(
            RunCleanupTarget::new(
                id(1).parse().unwrap(),
                id(2).parse().unwrap(),
                id(3).parse().unwrap(),
                persisted(),
                1,
                vec![lease(10, 20, "host-1", 7), other]
            ),
            Err(expected)
        );
    }
}

#[test]
fn foreign_run_lease_rejects() {
    let held = RunCleanupLeaseFence::new(
        id(99).parse().unwrap(),
        id(20).parse().unwrap(),
        id(10).parse().unwrap(),
        String::from("host-1"),
        7,
    )
    .unwrap();
    assert_eq!(
        RunCleanupTarget::new(
            id(1).parse().unwrap(),
            id(2).parse().unwrap(),
            id(3).parse().unwrap(),
            persisted(),
            1,
            vec![held]
        ),
        Err(RunCleanupError::LeaseRunMismatch)
    );
}

#[test]
fn maximum_complete_set_is_explicit_and_bounded() {
    let held: Vec<_> = (0..32)
        .map(|number| lease(100 + number, 200 + number, "host-1", 7))
        .collect();
    assert_eq!(
        target(persisted(), 1, held.clone()).leases().len(),
        MAX_RUN_CLEANUP_LEASES
    );
    let mut excess = held;
    excess.push(lease(132, 232, "host-1", 7));
    assert_eq!(
        RunCleanupTarget::new(
            id(1).parse().unwrap(),
            id(2).parse().unwrap(),
            id(3).parse().unwrap(),
            persisted(),
            1,
            excess
        ),
        Err(RunCleanupError::TooManyLeases)
    );
}

#[test]
fn planned_vm_still_requires_matching_destroy_or_absence() {
    let planned = target(
        RunCleanupVmTarget::planned(host(), VmId(String::from("vm-1"))).unwrap(),
        1,
        vec![],
    );
    for observation in [
        RunCleanupVmObservation::destroyed(host(), VmId(String::from("vm-2"))).unwrap(),
        RunCleanupVmObservation::authoritatively_absent(host(), VmId(String::from("vm-2")))
            .unwrap(),
    ] {
        assert_eq!(
            RunCleanupReceipt::new(planned.clone(), observation, OffsetDateTime::UNIX_EPOCH),
            Err(RunCleanupError::VmMismatch)
        );
    }
    let result = receipt(planned);
    assert_eq!(
        result.observation().kind(),
        RunCleanupVmObservationKind::Destroyed
    );
    assert_eq!(result.observed_at(), OffsetDateTime::UNIX_EPOCH);
}

mod digests;
mod host_scope;
