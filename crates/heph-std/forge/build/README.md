# Build providers

The Forge build providers execute the path from an exact repository commit to
an immutable OCI image and verified root filesystem. PostgreSQL adapters claim
durable jobs, local runtime adapters prepare checkouts and run Buildah, Trivy,
Syft, and Umoci in isolated VM phases, and the orchestrator imports the
resulting artifacts into a release.

The workers consume the provider-neutral contracts in `heph-build`. Their
shared handoff is explicit: a production job names the source revision,
context digest, Dockerfile, and approved base; a successful output carries its
manifest digest, scan, attestation, and SBOM evidence; materialization then
installs a root that the daemon can select by immutable image reference.

The crate guides below describe each provider boundary:

- [`oci-builder-postgres/`](oci-builder-postgres/) persists claims and
  publication intents.
- [`oci-builder-runtime-local/`](oci-builder-runtime-local/) performs local
  checkout, isolated image production, verification, and export.
- [`oci-builder-worker/`](oci-builder-worker/) runs durable production and
  rootfs materialization workers.
- [`orchestrator/`](orchestrator/) turns accepted build requests into release
  artifacts.
- [`postgres/`](postgres/) persists build execution and verification state.
