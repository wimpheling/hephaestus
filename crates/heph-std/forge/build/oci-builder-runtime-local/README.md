# Purpose

`oci-builder-runtime-local` performs the host-side work for an isolated OCI
build. It prepares an exact Git checkout, mounts the approved base and source
into builder and verifier VM phases, runs Buildah, Trivy, Syft, and Umoci, and
returns immutable publication material and a verified root filesystem.

# Responsibilities

`LocalOciRuntimeConfig::initialize` validates the configured binaries, roots,
and image layouts. `VmOciOperation` builds explicit VM specs with separated
source, base, candidate, scratch, and verification paths; the runtime seals
the candidate layout, normalizes it to one OCI index, scans it, and produces
attestation, SBOM, scan, manifest, and rootfs outputs.

Filesystem helpers create private directories, reject unsafe trees and
symlinks, and copy only verified output. Publication helpers send the
immutable layout through the Forge registry publisher and return the evidence
needed by the core build contract.

# When

Initialize `LocalOciRuntime` and `VmOciOperation` from the daemon's build
configuration, then let `OciImageProductionWorker` call the runtime for a
claimed `IsolatedOciBuild`. Use `verified_vm_publication_material` only after
the verifier phase has completed successfully.
