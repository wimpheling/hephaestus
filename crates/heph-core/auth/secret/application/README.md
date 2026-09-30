# Purpose

`secret-application` runs the secret workflow from an owner's first value to a
guest's exact runtime lease. It checks actor authority while creating,
rotating, granting, importing, and binding secrets, then gives dispatch and
broker code a metadata-only authority object for one run; plaintext is
released only at the final delivery step.

# Responsibilities

The create, rotate, grant, accept, and bind commands each carry a
`SecretCommandKey`, so retries of those commands replay one durable outcome.
Revoke, enable, and purge use separate lifecycle calls without a command key.
The service uses the caller identity while moving a value from its owner,
through an exact grant and import, into an immutable instance binding.

At dispatch, `SecretDispatchResolver` checks the instance revision, attachment,
phase, and expiry and returns lease metadata plus a fresh
`OpaqueRuntimeCredential`. `SecretRuntimeResolver::receive_raw` releases one
raw slot only after checking that credential. Every brokered request undergoes
the lease and destination checks; `https_v1` requests additionally undergo an
HTTPS-rule snapshot check. The adapter receives only the bounded operation it
needs, and its response is sanitized before returning to the guest.

# When

Use command and runtime ports at the transport and dispatch boundaries. A
dispatcher first receives lease metadata and a runtime credential, then asks
for one selected raw slot only when a raw mount is being built:

```rust
let authority = dispatch.resolve_for_dispatch(&identity, ResolveRunSecrets {
    command_key, session_id, run_id, instance_id, instance_revision_id,
    attachment_id, target_ref, target_commit, phase, expires_at,
}).await?;
let raw = runtime.receive_raw(&authority.credential, run_id, slot).await?;
```
