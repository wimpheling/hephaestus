#[cfg(test)]
use super::*;
use time::{Duration, OffsetDateTime};

#[test]
fn bearer_digests_are_stable_domain_separated_and_redacted() {
    let bytes = [0x5a; 32];
    let handoff = UiBrowserHandoffSecret::from_bytes(bytes);
    let session = UiBrowserSessionSecret::from_bytes(bytes);
    let handoff_digest = handoff.digest();
    let session_digest = session.digest();

    assert_eq!(handoff_digest, handoff.digest());
    assert_ne!(handoff_digest.as_bytes(), session_digest.as_bytes());
    assert_eq!(format!("{handoff:?}"), "UiBrowserHandoffSecret(REDACTED)");
    assert_eq!(
        format!("{handoff_digest:?}"),
        "UiBrowserHandoffDigest(REDACTED)"
    );
    assert_eq!(format!("{session_digest}"), "[redacted]");
    assert!(!format!("{handoff:?}").contains("5a"));
}

#[test]
fn child_expiry_is_capped_without_sliding_renewal() {
    let issued = OffsetDateTime::UNIX_EPOCH;
    let parent_later = issued + Duration::hours(24);
    assert_eq!(
        child_session_expiry(issued, parent_later).expect("valid parent"),
        issued + Duration::hours(12)
    );
    let parent_earlier = issued + Duration::hours(2);
    assert_eq!(
        child_session_expiry(issued, parent_earlier).expect("valid parent"),
        parent_earlier
    );
    assert_eq!(
        child_session_expiry(issued, issued),
        Err(UiBrowserSessionFailure::Unauthenticated)
    );

    let maximum =
        OffsetDateTime::new_in_offset(time::Date::MAX, time::Time::MAX, time::UtcOffset::UTC);
    let near_maximum = maximum
        .checked_sub(Duration::hours(1))
        .expect("representable test instant");
    assert_eq!(
        child_session_expiry(near_maximum, maximum),
        Err(UiBrowserSessionFailure::InvalidLifetime)
    );
}

#[test]
fn browser_route_rejects_urls_and_round_trips_only_typed_paths() {
    let route = UiBrowserRoute::parse("assets/app/main.js").expect("safe route");
    assert_eq!(route.as_str(), "assets/app/main.js");
    let encoded = serde_json::to_string(&route).expect("route JSON");
    assert_eq!(
        serde_json::from_str::<UiBrowserRoute>(&encoded).expect("typed route JSON"),
        route
    );
    for value in [
        "",
        "/absolute",
        "//other.example",
        "https://other.example/app",
        "../escape",
        "app?next=other",
        "app#fragment",
    ] {
        assert_eq!(
            UiBrowserRoute::parse(value),
            Err(UiBrowserRouteError::Invalid)
        );
        let json = format!("{value:?}");
        assert!(
            serde_json::from_str::<UiBrowserRoute>(&json).is_err(),
            "{value}"
        );
    }
}
