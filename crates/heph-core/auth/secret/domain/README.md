# Purpose

`secret-domain` describes a secret's complete authority lifecycle. It connects
an owner and immutable version to grants, imports, instance bindings, and
short-lived run or gateway leases, while giving every step the same delivery
policy and lifecycle vocabulary.

# Responsibilities

The domain keeps the owner, immutable version, source grant, target import,
instance binding, and runtime or gateway lease linked explicitly. Application
code can therefore resolve one exact authority for a run or invocation instead
of treating a secret name as permission to read a value.

`SecretUsePolicy` and the validation functions enforce delivery mode, execution
phase, destination, tenant boundary, lifecycle transition, and size rules.
`SecretValue` is redacted from formatting and serialization and best-effort
wiped on drop; `OpaqueRuntimeCredential` is exposed only at authentication
time and supplies the one-way hash used for storage.

# When

Use these types while accepting a secret command and selecting a runtime lease.
Validate policy first, then pass a `SecretValue` only to the narrow encryption
or delivery operation that needs it:

```rust
let value = SecretValue::new(bytes)?;
let policy = SecretUsePolicy { delivery_modes, phases, destinations }
    .normalized()?;
```
