//! New ordinary intervals use SQL precision before identity hashing.

use super::{OffsetDateTime, RunAuthorityError};

const MICROSECOND: i128 = 1000;

pub(super) fn interval(
    existing: Option<(OffsetDateTime, OffsetDateTime)>,
    new: impl FnOnce() -> (i128, i128),
) -> Result<(OffsetDateTime, OffsetDateTime), RunAuthorityError> {
    existing.map_or_else(
        || {
            let (now_ns, ttl_ns) = new();
            new_interval(now_ns, ttl_ns)
        },
        Ok,
    )
}

fn new_interval(
    now_ns: i128,
    ttl_ns: i128,
) -> Result<(OffsetDateTime, OffsetDateTime), RunAuthorityError> {
    if ttl_ns <= 0 {
        return Err(RunAuthorityError::redacted(
            "ordinary session TTL must be positive",
        ));
    }
    let now = timestamp(now_ns, "ordinary session issue time is out of range")?;
    let issued_ns = floor(now.unix_timestamp_nanos())?;
    let issued = timestamp(issued_ns, "ordinary session issue time is out of range")?;
    let expiry_ns = issued_ns
        .checked_add(ttl_ns)
        .ok_or_else(|| RunAuthorityError::redacted("ordinary session interval overflowed"))?;
    let expiry = timestamp(expiry_ns, "ordinary session expiry is out of range")?;
    let expires_ns = floor(expiry.unix_timestamp_nanos())?;
    if expires_ns <= issued_ns {
        return Err(RunAuthorityError::redacted(
            "ordinary session interval has no positive SQL precision",
        ));
    }
    let expires = timestamp(expires_ns, "ordinary session expiry is out of range")?;
    Ok((issued, expires))
}

fn timestamp(value: i128, message: &'static str) -> Result<OffsetDateTime, RunAuthorityError> {
    OffsetDateTime::from_unix_timestamp_nanos(value)
        .map_err(|_| RunAuthorityError::redacted(message))
}

fn floor(value: i128) -> Result<i128, RunAuthorityError> {
    value
        .div_euclid(MICROSECOND)
        .checked_mul(MICROSECOND)
        .ok_or_else(|| RunAuthorityError::redacted("ordinary SQL precision overflowed"))
}

#[cfg(test)]
#[path = "ordinary_time_tests.rs"]
mod tests;
