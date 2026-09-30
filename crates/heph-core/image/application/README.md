# Purpose

`builder-catalog-application` is the application port for reading approved OCI
image catalog entries and resolving a selection to immutable execution
provenance. Build, release, and runtime callers can use the same validation
boundary while the concrete catalog remains in its storage adapter.

# Responsibilities

`ImageCatalogApplication` validates metadata returned by an `ImageCatalog`,
resolves a stable key or immutable reference, and rejects unavailable, retired,
or platform-operation-only images. Its publication view validates the paired
registry state and supply-chain evidence after an authenticated caller is
provided. The service returns safe metadata and an immutable reference; it does
not turn catalog selection into network, secret, mount, or resource authority.
That policy remains with the caller that is preparing a build or run.

# When

Use this crate when a build or runtime workflow needs to turn a reviewed image
key into frozen provenance:

```rust
let resolved = catalog_application.resolve_key(&image_key).await?;
```

Persist the returned `ResolvedImage` with the release or run record so later
materialization uses the same catalog identity and policy version.
