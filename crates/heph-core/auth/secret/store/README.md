# Purpose

`secret-store` turns one immutable secret version into an authenticated,
durable envelope and turns that envelope back into a short-lived value after
the caller has authorized access. It uses a fresh data key per version and a
versioned host key to protect that data key.

# Responsibilities

`EncryptedStore::seal` creates a fresh AES-256-GCM data key and unique nonces
for each immutable version, wraps that data key with the configured host key,
and authenticates `VersionContext`. Owner, secret, version, sequence, and media
type therefore cannot be swapped silently in durable storage.

`resolve` looks up the exact referenced key and fails closed on missing keys,
context mismatch, tampering, unsupported algorithms, or invalid metadata.
Errors do not include plaintext, ciphertext, nonce, or key material; successful
resolution returns a redacted `SecretValue` only after the caller's lease
check.

# When

Call `seal` after the domain has accepted a new version, and call `resolve`
only after the application has checked the exact lease and lifecycle:

```rust
let store = EncryptedStore::new(keys);
let encrypted = store.seal(&context, &value)?;
let value = store.resolve(&context, &encrypted)?;
```
