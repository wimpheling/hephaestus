//! Exact trusted configuration and PostgreSQL-precision session bounds.

use super::super::agent_repository::PgRuntimeSessionRepository;
use runtime_authority::RuntimeAuthorityError;
use time::OffsetDateTime;

impl PgRuntimeSessionRepository {
    pub(in crate::runtime_authority) fn invocation_config_bytes(
        &self,
    ) -> Result<Vec<u8>, RuntimeAuthorityError> {
        // Existing Core hashes frame native usize lengths. This optional path
        // supports the reviewed 64-bit encoding without changing other paths.
        if usize::BITS != 64 || time::Date::MIN.year() != -9999 || time::Date::MAX.year() != 9999 {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        let configuration = self
            .invocation_configuration
            .as_ref()
            .ok_or(RuntimeAuthorityError::Unsupported)?;
        serde_json::to_vec(&configuration.canonical_json())
            .map_err(|_| RuntimeAuthorityError::Persistence)
    }
}

pub(super) fn interval(
    issued: OffsetDateTime,
    expires: OffsetDateTime,
) -> Result<(), RuntimeAuthorityError> {
    if issued.unix_timestamp_nanos().rem_euclid(1000) != 0
        || expires.unix_timestamp_nanos().rem_euclid(1000) != 0
        || expires <= issued
        || expires - issued > time::Duration::seconds(3600)
    {
        return Err(RuntimeAuthorityError::Unsupported);
    }
    Ok(())
}
