use super::*;

#[test]
fn digest_binds_vm_provenance_generation_and_entire_fence_set() {
    let base = target(persisted(), 1, vec![lease(10, 20, "host-1", 7)]);
    for changed in [
        target(persisted(), 2, base.leases().to_vec()),
        target(
            RunCleanupVmTarget::planned(host(), VmId(String::from("vm-1"))).unwrap(),
            1,
            base.leases().to_vec(),
        ),
        target(
            RunCleanupVmTarget::persisted(host(), VmId(String::from("vm-2"))).unwrap(),
            1,
            base.leases().to_vec(),
        ),
        target(persisted(), 1, vec![lease(10, 20, "host-1", 8)]),
        target(
            RunCleanupVmTarget::persisted(
                RunCleanupHostId::new(String::from("local-provider-1"), String::from("host-2"))
                    .unwrap(),
                VmId(String::from("vm-1")),
            )
            .unwrap(),
            1,
            vec![lease(10, 20, "host-2", 7)],
        ),
        target(persisted(), 1, vec![lease(10, 21, "host-1", 7)]),
        target(persisted(), 1, vec![lease(11, 20, "host-1", 7)]),
        target(persisted(), 1, vec![]),
    ] {
        assert_ne!(base.digest(), changed.digest());
        assert_eq!(
            receipt(base.clone()).matches_target(&changed),
            Err(RunCleanupError::TargetMismatch)
        );
    }
    for (run, instance, revision) in [(4, 2, 3), (1, 4, 3), (1, 2, 4)] {
        let changed = RunCleanupTarget::new(
            id(run).parse().unwrap(),
            id(instance).parse().unwrap(),
            id(revision).parse().unwrap(),
            persisted(),
            1,
            vec![],
        )
        .unwrap();
        assert_ne!(target(persisted(), 1, vec![]).digest(), changed.digest());
    }
}

#[test]
fn digest_encoding_has_a_stable_golden_vector() {
    // Independently encoded with fixed-width big-endian IDs/fences and u16
    // string lengths; this pins the representation for PostgreSQL adapters.
    let held = target(persisted(), 1, vec![lease(10, 20, "host-1", 7)]);
    assert_eq!(
        *held.digest(),
        [
            0x3a, 0x77, 0xc3, 0x7c, 0xb1, 0xb6, 0xee, 0xfe, 0x55, 0x2e, 0xe6, 0xc3, 0x09, 0xfe,
            0x47, 0xb6, 0x0d, 0xb1, 0x4b, 0xc8, 0x5b, 0x4f, 0x22, 0x30, 0x93, 0x45, 0xe2, 0xa0,
            0x85, 0x66, 0x39, 0x6e,
        ]
    );
}
