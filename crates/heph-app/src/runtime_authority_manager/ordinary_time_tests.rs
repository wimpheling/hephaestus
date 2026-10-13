use super::{OffsetDateTime, interval};

#[test]
fn fractional_clock_and_ttl_floor_before_hashing_without_extending_ttl() {
    let ttl = 2_345_678;
    let (issued, expires) = interval(None, || (1_234_567, ttl)).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), 1_234_000);
    assert_eq!(expires.unix_timestamp_nanos(), 3_579_000);
    assert!(expires.unix_timestamp_nanos() - issued.unix_timestamp_nanos() <= ttl);
}

#[test]
fn exact_interval_and_long_ttl_have_no_invocation_cap_or_lease_clamp() {
    let ttl = 7_200_000_000_000;
    let (issued, expires) = interval(None, || (1_000_000, ttl)).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), 1_000_000);
    assert_eq!(expires.unix_timestamp_nanos(), 1_000_000 + ttl);
}

#[test]
fn negative_epoch_floor_is_euclidean_and_duration_stays_within_ttl() {
    let (issued, expires) = interval(None, || (-1, 2001)).unwrap();
    assert_eq!(issued.unix_timestamp_nanos(), -1000);
    assert_eq!(expires.unix_timestamp_nanos(), 1000);
    assert!(expires.unix_timestamp_nanos() - issued.unix_timestamp_nanos() <= 2001);
}

#[test]
fn nonpositive_and_submicrosecond_ttl_deny_despite_clock_fraction() {
    for now in [-1, 0, 999, 1001] {
        for ttl in [-1, 0, 1, 999] {
            assert!(interval(None, || (now, ttl)).is_err());
        }
    }
}

#[test]
fn minimum_positive_precision_and_fractional_ttl_are_conservative() {
    for ttl in [1000, 1999] {
        let (issued, expires) = interval(None, || (1999, ttl)).unwrap();
        assert_eq!(issued.unix_timestamp_nanos(), 1000);
        assert_eq!(expires.unix_timestamp_nanos(), 2000);
        assert!(expires.unix_timestamp_nanos() - issued.unix_timestamp_nanos() <= ttl);
    }
}

#[test]
fn addition_overflow_and_raw_timestamp_range_errors_deny() {
    assert!(interval(None, || (1000, i128::MAX)).is_err());
    assert!(interval(None, || (i128::MIN, 1000)).is_err());
    assert!(interval(None, || (i128::MAX, 1000)).is_err());
    let last =
        OffsetDateTime::new_in_offset(::time::Date::MAX, ::time::Time::MAX, ::time::UtcOffset::UTC)
            .unix_timestamp_nanos();
    // Flooring must not hide an expiry outside time's representable range.
    assert!(interval(None, || (last, 1000)).is_err());
    assert!(interval(None, || (0, last + 1)).is_err());
}

#[test]
fn existing_interval_is_verbatim_and_never_evaluates_new_clock_or_ttl() {
    for nanos in [(-1, 10_000_009), (1000, 2000)] {
        let existing = (
            OffsetDateTime::from_unix_timestamp_nanos(nanos.0).unwrap(),
            OffsetDateTime::from_unix_timestamp_nanos(nanos.1).unwrap(),
        );
        assert_eq!(
            interval(Some(existing), || panic!(
                "existing interval must not evaluate new inputs"
            ))
            .unwrap(),
            existing
        );
    }
}
