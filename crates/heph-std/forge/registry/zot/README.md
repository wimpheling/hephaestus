# Purpose

`registry-zot` is the authenticated Zot client used by registry reconciliation.
It inspects one immutable manifest reference and returns the platform and
supply-chain evidence needed to decide whether a catalog publication is
available.

# Responsibilities

The client addresses only its configured private authority, obtains a
namespace-scoped pull token, refuses redirects, and applies bounded request
timeouts and response sizes. It fetches the exact digest, validates that the
subject is an image index, checks platform descriptors, and verifies each
recognized referrer points back to that subject with the expected media type.
Invalid graphs are reported as invalid inspections; authority, configuration,
authorization, and availability failures remain distinct so reconciliation can
retry safely.

# When

Configure the client with the Zot authority and a pull-token provider, then
hand it to the registry reconciliation port:

```rust
let registry = ZotHttpRegistry::new(config, token_provider)?;
```

The caller receives a present, missing, or invalid inspection and should persist
that result with the publication's immutable digest and policy state.
