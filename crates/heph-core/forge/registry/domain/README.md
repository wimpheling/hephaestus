# Purpose

`registry-domain` gives Forge ownership and publication decisions for OCI
content a durable, typed shape. It connects an owner to a canonical registry
namespace and tracks whether an immutable manifest has been requested,
verified, approved, retired, or found missing.

# Responsibilities

`RegistryNamespace` and `NamespaceClaim` bind a project, release agent, or
platform image to its canonical publication path; application authorization must
still decide whether that owner may publish. `ImmutableManifestReference` and
OCI descriptors identify the exact content, while `PublicationIntent`
models the retryable lifecycle from pending publication through read-back
verification and approval.

Supply-chain evidence records the policy version, platforms, referrers, scan,
attestation, and SBOM references required for a `VerifiedPublication`.
Inventory and retention types let operations compare durable ownership and
publication state with registry content before removing anything.

# When

Use this crate when creating a publication intent, verifying the registry's
immutable manifest, deciding whether content is safe for runtime selection, or
building a retention report. Pass the resulting typed intent or evidence to
the registry worker and reconciliation ports.
