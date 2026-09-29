# Purpose

`secret-key-local` loads host-local versioned key material for the encrypted
secret store. It provides the active wrapping key for new secret versions while
retaining older keys so existing versions can still be resolved.

# Responsibilities

`LocalKeyProvider::new` validates unique references, exact 32-byte keys, and an
available active reference. `from_directory` additionally requires an absolute
service-owned directory with mode `0700` and regular key files with mode `0400`.
`rotate_active_key` selects a retained key for later seals, while
`remove_key` refuses to remove the active key.

# When

Load the provider during host startup with `from_directory` or construct it
from already validated key bytes, then pass it to `EncryptedStore`. Rotate the
active reference only after the replacement key is present; retain old keys
until every encrypted version that needs them has been migrated or purged.
