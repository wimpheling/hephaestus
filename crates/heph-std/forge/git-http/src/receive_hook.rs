//! Trusted inspection for Git's quarantined `pre-receive` boundary.
//!
//! Git invokes `pre-receive` after unpacking incoming objects into quarantine
//! and before its atomic ref transaction. This adapter treats hook commands as
//! proposals only: it verifies every old object against canonical refs and
//! derives transitions and paths from Git's object database before applying
//! the capability grammar.

mod commands;
mod errors;
mod inspection;
mod repository;

use crate::receive_policy::{
    CapabilityReceivePolicyGuard, ReceivePolicyGuard, ResolvedRuntimeReceiveContext,
    TrustedReceiveProposal,
};
use commands::parse_commands;
use git_capability_domain::RepositoryId;
use inspection::inspect_update;
use std::{io::BufRead, path::Path};

pub use errors::ReceiveHookError;

/// Host-owned inputs for one quarantined receive inspection.
#[derive(Debug)]
pub struct ReceiveHookInput<'a> {
    /// Canonical bare repository selected by the trusted HTTP route.
    pub repository_path: &'a Path,
    /// Repository identifier selected by the trusted HTTP route.
    pub repository_id: RepositoryId,
    /// Quarantine created by `git-receive-pack` for this transaction.
    pub quarantine_path: &'a Path,
    /// Actual HTTP bytes, already bounded by the host transport.
    pub request_bytes: u64,
    /// Current host wall-clock time used for the final expiry check.
    pub now_unix_seconds: i64,
}

/// Inspects and authorizes a complete `pre-receive` command stream.
///
/// Success means the exact proposal may proceed to Git's canonical atomic ref
/// transaction. Failure must be returned as a non-zero hook exit status.
///
/// # Errors
///
/// Returns a fail-closed error if repository facts cannot be derived, the
/// transaction exceeds its immutable bounds, or capability policy denies any
/// update.
pub fn authorize_quarantined_receive(
    context: ResolvedRuntimeReceiveContext,
    input: &ReceiveHookInput<'_>,
    commands: impl BufRead,
) -> Result<TrustedReceiveProposal, ReceiveHookError> {
    if context.repository_id() != input.repository_id
        || !context.is_active_at(input.now_unix_seconds)
    {
        return Err(ReceiveHookError::ContextMismatch);
    }
    let repository = repository::canonical_repository(input.repository_path)?;
    let quarantine = repository::canonical_quarantine(&repository, input.quarantine_path)?;
    let limits = context.transfer_limits();
    if input.request_bytes > limits.request_bytes() {
        return Err(ReceiveHookError::TransferLimitExceeded);
    }
    let (pack_bytes, object_count) =
        repository::quarantine_stats(&repository, &quarantine, limits.object_count())?;
    if pack_bytes > limits.pack_bytes() || object_count > limits.object_count() {
        return Err(ReceiveHookError::TransferLimitExceeded);
    }
    let commands = parse_commands(commands, limits.ref_updates())?;
    let mut updates = Vec::with_capacity(commands.len());
    for command in commands {
        updates.push(inspect_update(&repository, &quarantine, command)?);
    }
    let proposal = TrustedReceiveProposal::new(
        context,
        updates,
        input.request_bytes,
        pack_bytes,
        object_count,
    );
    CapabilityReceivePolicyGuard
        .authorize(&proposal)
        .map_err(ReceiveHookError::Policy)?;
    Ok(proposal)
}
