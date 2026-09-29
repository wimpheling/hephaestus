# Purpose

`runtime-handoff-local` provides encrypted, crash-recoverable handoff files for
short-lived runtime credentials on one trusted VM host. It implements the core
handoff ports without putting bearer plaintext in durable envelopes.

# Responsibilities

`EncryptedFileHandoffStore` encrypts generic runtime credentials with
AES-256-GCM, binds the session, generation, and expiry as authenticated data,
and restricts its directory and files to the host owner. The separate
`EncryptedFileRuntimeGitHandoffStore` uses its own filename extension and the
same exact-generation redelivery, destruction, and expiry-purge lifecycle.

# When

Create the appropriate store under a private host directory and pass it to the
runtime authority issuer. Call `open` only for the matching session generation
during bootstrap redelivery; destroy the envelope after acknowledgement or
revocation and run expiry recovery after restarts.
