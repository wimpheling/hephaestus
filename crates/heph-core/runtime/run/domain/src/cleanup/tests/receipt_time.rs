use super::*;

#[test]
fn submicrosecond_observations_round_trip_at_database_precision() {
    for timestamp in [-1_234_567_891_i128, 1_234_567_891_i128] {
        let input = OffsetDateTime::from_unix_timestamp_nanos(timestamp).unwrap();
        let exact = target(persisted(), 1, vec![]);
        let observation = RunCleanupVmObservation::destroyed(host(), VmId("vm-1".into())).unwrap();
        let receipt = RunCleanupReceipt::new(exact.clone(), observation.clone(), input).unwrap();
        let stored = receipt.observed_at();
        assert_eq!(stored.nanosecond() % 1_000, 0);
        assert!(stored <= input);
        assert!((input - stored).whole_nanoseconds() < 1_000);
        let decoded = OffsetDateTime::from_unix_timestamp_nanos(
            stored.unix_timestamp_nanos() / 1_000 * 1_000,
        )
        .unwrap();
        assert_eq!(
            RunCleanupReceipt::new(exact, observation, decoded).unwrap(),
            receipt
        );
    }
}
