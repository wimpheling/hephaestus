//! Exact terminal cleanup after Core has confirmed the original guest stopped.
//!
//! Complete independent observations precede either retained-store deletion.
//! These comparisons grant no physical stop or user/dispatch authority.

use super::{RunAuthorityError, authority_error, runtime_git_authority_error};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus};
use runtime_authority::{
    ExpiredRuntimeHandoff, RuntimeHandoffStore, RuntimeSessionIssuer, RuntimeSessionRepository,
};
use runtime_git_authority::{
    ExpiredRuntimeGitHandoff, RuntimeGitCredentialIssuer, RuntimeGitCredentialRepository,
    RuntimeGitHandoffStore,
};
use uuid::Uuid;

#[cfg(test)]
#[path = "cleanup/tests.rs"]
mod tests;

pub(super) async fn expired<R, H, G, J>(
    issuer: &RuntimeSessionIssuer<R, H>,
    git_issuer: &RuntimeGitCredentialIssuer<G, J>,
    run: Uuid,
    session: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
) -> Result<(), RunAuthorityError>
where
    R: RuntimeSessionRepository,
    H: RuntimeHandoffStore,
    G: RuntimeGitCredentialRepository,
    J: RuntimeGitHandoffStore,
{
    let original = issuer
        .observe_expired_run_handoff(run, session, generation)
        .await
        .map_err(authority_error)?;
    let git_original = git_issuer
        .observe_expired_run_handoff(run, session, generation)
        .await
        .map_err(runtime_git_authority_error)?;
    compare(&original, &git_original)?;
    // A Git failure after generic removal is partial cleanup, never success.
    // Owning manager retains BOTH issuers/stores. Retry freshly observes both
    // originals again before invoking their exact idempotent deletion delegates.
    issuer
        .destroy_expired_handoff(&original)
        .map_err(authority_error)?;
    git_issuer
        .destroy_expired_handoff(&git_original)
        .map_err(runtime_git_authority_error)
}

fn compare(
    original: &ExpiredRuntimeHandoff,
    git: &ExpiredRuntimeGitHandoff,
) -> Result<(), RunAuthorityError> {
    let session = original.session();
    let ceiling = git.ceiling();
    let other = ceiling.session();
    if original.pins().run() != ceiling.run()
        || session.id != other.session
        || session.snapshot_id != other.snapshot
        || session.identity_hash != other.identity_hash
        || session.generation != other.generation
        || session.status != RuntimeSessionStatus::Expired
        || session.status != other.status
        || session.issued_at != other.issued_at
        || session.expires_at != other.expires_at
        || session.acknowledged_at != other.acknowledged_at
        || session.revoked_at != other.revoked_at
        || original.snapshot() != ceiling.snapshot()
        || original.identity() != ceiling.identity()
        || original.credential_hash() != ceiling.generic_credential_hash()
        || original.observed_at() < session.expires_at
        || ceiling.observed_at() < session.expires_at
    {
        return Err(RunAuthorityError::redacted(
            "expired runtime handoff ceilings differ",
        ));
    }
    // CoreGit intentionally represents only Run UUID, not the other five Run
    // pins or attachment. Its trusted Pg observer rechecks all six and attachment
    // against its separately requalified complete generic original before return.
    Ok(())
}
