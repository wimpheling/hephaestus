//! Actual accepted mailbox birth metadata; no delegated user or physical proof.

#[path = "legacy_vm_placement/claims.rs"]
mod claims;
#[path = "postgres/support.rs"]
// Reuse the real accepted event fixture; unrelated recovery helpers are unused.
#[allow(dead_code)]
mod fixture;
#[path = "execution_profiles/support.rs"]
mod support;
