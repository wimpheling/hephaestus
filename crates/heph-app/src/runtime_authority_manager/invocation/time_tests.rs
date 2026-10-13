use super::interval;

#[test]
fn negative_nanoseconds_floor_toward_negative_infinity() {
    let (issued, expires) = interval(-1, 2000, std::iter::empty()).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), -1000);
    assert_eq!(expires.unix_timestamp_nanos(), 1000);
    let (issued, expires) = interval(-3001, 10_000, std::iter::once(-1)).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), -4000);
    assert_eq!(expires.unix_timestamp_nanos(), -1000);
}

#[test]
fn expiry_is_capped_by_every_lease_independently_of_order() {
    for leases in [[99_999, 55_555], [55_555, 99_999]] {
        let (issued, expires) = interval(9999, 7_200_000_000_000, leases.into_iter()).unwrap();
        assert_eq!(issued.unix_timestamp_nanos(), 9000);
        assert_eq!(expires.unix_timestamp_nanos(), 55_000);
        assert!(
            leases
                .iter()
                .all(|deadline| expires.unix_timestamp_nanos() <= *deadline)
        );
    }
}

#[test]
fn empty_interval_input_retains_the_exact_one_hour_cap() {
    for ttl in [3_600_000_000_000, 7_200_000_000_000, i128::MAX] {
        let (issued, expires) = interval(1, ttl, std::iter::empty()).unwrap();
        assert_eq!(issued.unix_timestamp_nanos(), 0);
        assert_eq!(expires.unix_timestamp_nanos(), 3_600_000_000_000);
    }
}

#[test]
fn nonpositive_and_precision_collapsed_intervals_are_denied() {
    for ttl in [-1, 0, 1, 999] {
        assert!(interval(1001, ttl, std::iter::empty()).is_err());
    }
    assert!(interval(1001, 1000, std::iter::once(1999)).is_err());
    assert!(interval(1001, 1000, std::iter::once(1000)).is_err());
    let (issued, expires) = interval(1001, 1000, std::iter::once(2000)).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), 1000);
    assert_eq!(expires.unix_timestamp_nanos(), 2000);
}

#[test]
fn overflow_and_timestamp_range_errors_are_denied() {
    assert!(interval(i128::MIN, 1000, std::iter::empty()).is_err());
    assert!(interval(i128::MAX - 500, 10_000, std::iter::empty()).is_err());
    let last = ::time::OffsetDateTime::new_in_offset(
        ::time::Date::MAX,
        ::time::Time::MAX,
        ::time::UtcOffset::UTC,
    )
    .unix_timestamp_nanos();
    assert!(interval(last, 10_000, std::iter::empty()).is_err());
    assert!(interval(0, 1000, std::iter::once(i128::MIN)).is_err());
}
