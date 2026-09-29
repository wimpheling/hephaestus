# Purpose

`capability-domain` turns a release's declared capability into an exact
authority ceiling for one workload. A requirement names a slot and permitted
operations, a binding selects one concrete resource, and an authorization
snapshot freezes those choices for a runtime session.

# Responsibilities

When a release declares a capability, `CapabilityRequirement::new` checks that
its operations are valid for the resource kind. Later,
`CapabilityBinding::bind` selects one concrete resource and rejects grants
that exceed the declaration. The resulting snapshot is the frozen authority
ceiling used by the runtime.

Normalized requirements, bindings, and snapshots are hashed so the runtime
can prove that its identity matches the authority issued for its workload
revision. The bearer in `RuntimeCredential` is redacted from logs and
serialization; durable state receives only `RuntimeCredentialHash`, while
`RuntimeAuthority::permits_at` checks the frozen grant and expiry.

# When

Use it while compiling a release and again when the host checks a guest
operation. The same binding and snapshot connect the declaration to the live
decision:

```rust
let requirement = CapabilityRequirement::new(
    requirement_id, slot, CapabilityResourceKind::Repository,
    [CapabilityOperation::GitRead], [], true,
)?;
let binding = CapabilityBinding::bind(binding_id, &requirement, resource,
    [CapabilityOperation::GitRead])?;
let allowed = snapshot.allows(binding.resource(), CapabilityOperation::GitRead);
```
