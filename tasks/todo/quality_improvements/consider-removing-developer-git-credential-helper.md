# Consider removing the developer Git credential helper crate

Owner: unassigned

## Purpose

Decide whether the `git-credential-hephaestus` crate still has a supported
product role. If it does not, remove the crate and its stale documentation
references in a follow-up change.

## Current evidence

- The crate is a workspace member and provides the optional developer-machine
  `git-credential-hephaestus` helper for locally storing Hephaestus PATs.
- [`docs/git-credentials.md`](../../../docs/git-credentials.md) and the
  crate README document manual installation and Git configuration, but no
  repository runtime or packaging path invokes this helper.
- The active runtime Git path uses the separate packaged
  `heph-git-credential` binary from `crates/heph-std/runtime/vm/libkrun`; it
  reads an exact runtime authority and must not be removed or conflated with
  this developer helper.

## Decision checklist

- [ ] Confirm whether any supported developer workflow, release artifact, or
      external integration still installs or invokes the helper.
- [ ] Confirm the replacement developer Git authentication flow, if the helper
      is removed, and update user-facing documentation accordingly.
- [ ] If obsolete, remove the crate, its lockfile entry, README references,
      and `docs/git-credentials.md` instructions in one reviewed change.
- [ ] Verify that the runtime `heph-git-credential` binary and its integration
      tests remain intact.

## Completion evidence

Record the usage search, the chosen developer authentication flow, affected
documentation, and verification that runtime Git credential injection still
works before moving this task to `tasks/done/`.
