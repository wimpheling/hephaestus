# Purpose

`runtime-authority` delivers the authority prepared for one exact runtime to
trusted bootstrap code. It records a session's immutable snapshot and
identity, creates a short-lived bearer for handoff, and makes retries return
the same pending session instead of issuing a second authority.

# Responsibilities

`RuntimeSessionIssuer::issue` coordinates the durable repository and handoff
store. One session identity, snapshot, generation, and expiry produce one
pending bearer, including when the worker retries after a partial failure.
The repository keeps the credential hash and safe `StoredRuntimeSession`
metadata; plaintext is returned only to the trusted bootstrap step.

Acknowledging the exact generation deletes the handoff, as does revocation.
`recover_expired` cleans up elapsed sessions and envelopes after a worker or
host restart, limiting the lifetime of runtime authority even when normal
shutdown did not run.

`LiveRunSnapshotResolver` provides a separate read-only check for exact owned
run pins. Pre-issuance checks carry no session; issued checks carry the stable
metadata and ceiling of an already persisted session. Fresh acknowledgement can
change pending to active without changing that ceiling. Issued checks during
owned drain grant no acquisition, Start, renewal, or credential issuance.

This port checks live released-source use and generic runtime capabilities.
Caller execution rights, typed volume grants, and secret authorization remain
separate checks. Request DTOs and immutable snapshots confer no authority alone.

# When

Call the issuer after constructing an immutable `AuthorizationSnapshot` and
`RuntimeSessionIdentity`, immediately before trusted guest bootstrap. The
acknowledgement closes the one-time handoff:

```rust
let issued = issuer.issue(&snapshot, &identity, attachment_id, now).await?;
bootstrap.send_runtime_credential(issued.credential.expose()).await?;
issuer.acknowledge(identity.id(), RuntimeCredentialGeneration::INITIAL, now).await?;
```
