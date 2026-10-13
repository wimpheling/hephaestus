//! Provisioning-only creation or exact pending replay, with no Git credential.

use capability_domain::{RuntimeInvocation, RuntimeSessionIdentity, RuntimeSessionStatus};
use heph_run::{PreparedRunAuthority, RunState};
use runtime_authority::RuntimeSessionRepository;

use super::{
    OffsetDateTime, PgRunAuthorityManager, Run, RunAuthorityError, RuntimeCredentialGeneration,
    RuntimeSessionId, authority_error, denied, live, time,
};

impl PgRunAuthorityManager {
    pub(in crate::runtime_authority_manager) async fn prepare_invocation(
        &self,
        run: &Run,
    ) -> Result<PreparedRunAuthority, RunAuthorityError> {
        if run.state != RunState::Provisioning {
            return Err(denied("Invocation issuance requires actual Provisioning"));
        }
        let snapshot = live::snapshot(&self.repository, run).await?;
        let session_id = RuntimeSessionId::from_uuid(run.id.as_uuid());
        // find errors/orphans/withdrawal are not absence. B1 validates pending
        // replay; recorded_live_session_ceiling already rejects invalid history.
        let existing = self
            .repository
            .find(session_id)
            .await
            .map_err(authority_error)?;
        let lease_expiries = self.invocation_lease_expiries(run).await?;
        let now = OffsetDateTime::now_utc();
        let (issued_at, expires_at) = if let Some(session) = &existing {
            if session.status != RuntimeSessionStatus::PendingHandoff
                || session.generation != RuntimeCredentialGeneration::INITIAL
                || session.id != session_id
                || session.snapshot_id != snapshot.id()
                || session.revoked_at.is_some()
                || session.acknowledged_at.is_some()
                || session.issued_at > now
                || session.expires_at <= now
                || lease_expiries
                    .iter()
                    .any(|expiry| session.expires_at > *expiry)
            {
                return Err(denied(
                    "Invocation session is not an exact current pending replay",
                ));
            }
            (session.issued_at, session.expires_at)
        } else {
            time::interval(
                now.unix_timestamp_nanos(),
                self.session_ttl.whole_nanoseconds(),
                lease_expiries
                    .iter()
                    .copied()
                    .map(OffsetDateTime::unix_timestamp_nanos),
            )?
        };
        // New times are already SQL microseconds BEFORE Core's identity hash.
        // Replay retains the stored interval exactly; never derive another TTL.
        let identity = RuntimeSessionIdentity::new(
            session_id,
            snapshot.principal(),
            RuntimeInvocation::Run(run.id),
            &snapshot,
            issued_at,
            expires_at,
        )
        .map_err(|_| denied("Invocation runtime identity is invalid"))?;
        if existing
            .as_ref()
            .is_some_and(|session| session.identity_hash != identity.normalized_hash())
        {
            return Err(denied("Invocation pending identity hash changed"));
        }
        let issued = self
            .issuer
            .issue(&snapshot, &identity, None, now)
            .await
            .map_err(authority_error)?;
        if issued.session.id != session_id
            || issued.session.generation != RuntimeCredentialGeneration::INITIAL
            || issued.session.snapshot_id != snapshot.id()
            || issued.session.identity_hash != identity.normalized_hash()
            || issued.session.issued_at != issued_at
            || issued.session.expires_at != expires_at
            || issued.session.status != RuntimeSessionStatus::PendingHandoff
        {
            return Err(denied("Invocation issuer returned a different bootstrap"));
        }
        Ok(PreparedRunAuthority {
            bootstrap: Some(heph_runtime::RuntimeAuthorityBootstrap::new(
                issued.session.id.as_uuid(),
                issued.session.generation.get(),
                *issued.credential.expose(),
            )),
        })
    }
}
