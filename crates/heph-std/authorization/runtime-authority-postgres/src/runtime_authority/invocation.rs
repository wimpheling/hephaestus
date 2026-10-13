//! Explicit qualified119 worker boundary; recorded invoker IDs are data.

mod configuration;
mod context;
mod mounts;
mod pre_acquisition;
mod reservations;
mod sessions;

use super::{agent_repository::PgRuntimeSessionRepository, common::storage};
use runtime_authority::RuntimeAuthorityError;
use uuid::Uuid;

impl PgRuntimeSessionRepository {
    pub(super) async fn qualified_invocation_run(
        &self,
        run: Uuid,
    ) -> Result<bool, RuntimeAuthorityError> {
        let kind: Option<String> = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
            .bind(run)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        if !invocation_kind(kind.as_deref())? {
            return Ok(false);
        }
        self.invocation_config_bytes()?;
        Ok(true)
    }
}

// The ordinary source query contains an ELSE Update projection, so unknown
// persisted producer kinds must never reach it. Missing rows keep the ordinary
// lookup's existing absence handling; no producer authority follows from false.
fn invocation_kind(kind: Option<&str>) -> Result<bool, RuntimeAuthorityError> {
    match kind {
        None | Some("normal" | "update") => Ok(false),
        Some("invocation") => Ok(true),
        Some(_) => Err(RuntimeAuthorityError::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::invocation_kind;
    use runtime_authority::RuntimeAuthorityError;

    #[test]
    fn discriminator_preserves_known_kinds_and_denies_unknown_fallback() {
        for kind in [None, Some("normal"), Some("update")] {
            assert!(!invocation_kind(kind).unwrap());
        }
        assert!(invocation_kind(Some("invocation")).unwrap());
        for kind in ["", "Invocation", "future_producer"] {
            assert!(matches!(
                invocation_kind(Some(kind)),
                Err(RuntimeAuthorityError::Unsupported)
            ));
        }
    }
}
