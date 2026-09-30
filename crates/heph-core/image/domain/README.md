# Purpose

`builder-catalog-domain` models the immutable OCI image catalog used by
Hephaestus builds and guests. It gives callers stable image keys and IDs,
validated references, availability and role decisions, architecture and
toolchain metadata, and supply-chain provenance.

# Responsibilities

`OciImage` validates the catalog record and resolves only images that are
available for tenant execution. Platform-operation images, unavailable entries,
and retired entries produce explicit selection errors. Registry publication
values pair an immutable manifest with architecture, SBOM, provenance, scan,
and signature evidence; their state must agree with the consumer-visible
availability. These checks keep mutable tags and invalid publication
projections out of workflow records. Required evidence is enforced when the
publication workflow verifies the registry result.

# When

Use this crate in a catalog adapter or application port before persisting an
image decision:

```rust
image.validate()?;
let resolved = image.resolve()?;
```

Store the resolved immutable reference and policy version with the workflow
that selected the image.
