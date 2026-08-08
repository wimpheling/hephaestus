# Draft: Repository-owned Git session protocol

Owner: unassigned

## Purpose

Define the repository/release-owned contract for Git-backed interactive session
history before a chat release relies on Git commits as trustworthy records.

This is intentionally a focused follow-up to MVP 06, not a prerequisite for
the current gateway, credential, mailbox, or storage work. It should remain a
draft until an interactive chat release is prioritized for implementation.

## Problem

A normal fast-forward Git push can add a new commit that edits or deletes a
previous session record. Git commit author metadata is user-supplied and cannot
prove whether a Hephaestus user, a runtime, or another writer created a record.

## Proposed direction

The selected chat repository and released agent define their own versioned
session protocol:

- Existing session record files are immutable.
- Each new record has a unique stable ID and appears only in an allowed actor
  namespace.
- User-input and agent-output writers follow the repository's own contract.
- Agent records may use Hephaestus runtime attribution as evidence, but the
  repository/release defines the record format and interpretation.
- Content references are explicit and authorization-checked.

Git remains the durable history store. Hephaestus provides only generic secure
repository access: exact repository/ref/path capability checks, authenticated
receive attribution, and normal Git transition rules. It does not validate or
interpret session-record semantics.

## Scope when prioritized

- [ ] Define the session record layout, immutable fields, actor namespaces,
  stable record IDs, and content-reference rules.
- [ ] Define release-owned validation and use of generic Git attribution.
- [ ] Define how the release rejects or repairs mutation/deletion of old
  records, duplicate IDs, forged actor/correlation IDs, and unauthorized
  content references.
- [ ] Define authorized repair, tombstone, and export behavior without silently
  rewriting prior records.
- [ ] Define ordering, retry, and conflict behavior for concurrent writers.
- [ ] Add adversarial real-Git tests for history mutation, forged attribution,
  duplicate records, retry, and concurrent writers.
- [ ] Update MVP 06 completion evidence with the selected integrity protocol.

## Non-goals

This draft does not implement interactive sessions, guarantee physical erasure
from Git objects or forks, or define attachment storage. Those remain separate
decisions.
