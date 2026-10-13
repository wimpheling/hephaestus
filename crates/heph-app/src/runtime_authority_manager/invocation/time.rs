//! Pure checked SQL-microsecond interval calculation, before Core hashing.

use super::{OffsetDateTime, RunAuthorityError, denied};

const MICROSECOND: i128 = 1000;
const MAX_TTL: i128 = 3600 * 1_000_000_000;

pub(super) fn interval(
    now_ns: i128,
    ttl_ns: i128,
    lease_expiries: impl Iterator<Item = i128>,
) -> Result<(OffsetDateTime, OffsetDateTime), RunAuthorityError> {
    if ttl_ns <= 0 {
        return Err(denied("Invocation session TTL must be positive"));
    }
    let issued_ns = floor(now_ns)?;
    let mut expires_ns = issued_ns
        .checked_add(ttl_ns.min(MAX_TTL))
        .ok_or_else(|| denied("Invocation session interval overflowed"))?;
    for expiry in lease_expiries {
        expires_ns = expires_ns.min(expiry);
    }
    let expires_ns = floor(expires_ns)?;
    if expires_ns <= issued_ns {
        return Err(denied(
            "Invocation session interval has no positive SQL precision",
        ));
    }
    let issued = OffsetDateTime::from_unix_timestamp_nanos(issued_ns)
        .map_err(|_| denied("Invocation session issue time is out of range"))?;
    let expires = OffsetDateTime::from_unix_timestamp_nanos(expires_ns)
        .map_err(|_| denied("Invocation session expiry is out of range"))?;
    Ok((issued, expires))
}

fn floor(value: i128) -> Result<i128, RunAuthorityError> {
    value
        .div_euclid(MICROSECOND)
        .checked_mul(MICROSECOND)
        .ok_or_else(|| denied("Invocation SQL precision overflowed"))
}

#[cfg(test)]
#[path = "time_tests.rs"]
mod tests;
