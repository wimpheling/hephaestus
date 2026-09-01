# Local repository-owned OCI image builds

Repository images are project resources prepared from one exact pushed Git
revision. They are not arbitrary host builds or generic agent runs. The
project's **Images** tab shows the authorized definition, current state, and a
bounded redacted phase history; it never shows checkout paths, registry
credentials, or raw builder/verifier output.

## Explicit local operator workflow

Normal `cargo dev run` startup never builds, scans, publishes, or enables this
workflow. First inspect its state:

```sh
cargo dev repository-images status
```

An enabled workflow also needs two host-controlled OCI clients: `skopeo` for
the exact image read-back and `oras` for bounded registry artifact operations.
They run only in the local supervisor after the separate builder and verifier
VMs finish; neither executable, nor a registry credential, is made available
inside either VM or to the daemon. Install both before restarting an enabled
workflow. On Fedora, install `skopeo` through DNF. If the distribution does
not package ORAS, use the reviewed ORAS version and checksum pinned by
[`platform/build-tools/Dockerfile`](../platform/build-tools/Dockerfile), place
the verified executable on `PATH`, and confirm it with `oras version`.

For an intentionally non-standard local install, point the supervisor at the
two absolute executable paths instead of changing `workflow.env`:

```sh
export HEPHAESTUS_SKOPEO=/absolute/path/to/skopeo
export HEPHAESTUS_ORAS=/absolute/path/to/oras
```

The values are inspected at startup; this does not grant either client to a
repository image build.

The daemon and its libkrun workers need a soft open-file limit of at least
`8192`: Umoci needs that bounded capacity to export a full Ubuntu rootfs in the
isolated verifier VM. `cargo dev run` raises its inherited soft limit to that
value when the shell's hard limit permits it. For a managed daemon, configure
the service manager with `LimitNOFILE=8192` (or a higher reviewed value) before
starting it; do not try to alter a running guest's limits.

The command distinguishes a disabled workflow from an enabled workflow whose
installed catalog, OCI layouts, immutable tags, or execution-base manifest are
missing. Fix those prerequisites before starting a daemon; do not hand-edit
`workflow.env`.

An operator creates and reviews a platform release explicitly. Replace the
source, commit, and timestamp with the reviewed immutable release evidence:

```sh
cargo dev platform-images build \
  --source https://example.invalid/hephaestus \
  --revision <40-or-64-lowercase-commit-sha> \
  --created 2026-01-01T00:00:00Z
```

This is intentionally a heavy operation: it constructs, scans, and attests
four execution bases plus `oci-builder-ubuntu` and `oci-verifier-ubuntu` in a
fresh private release directory. A fixable HIGH or CRITICAL finding stops the
operation before publication and retains its policy report beside the failed
release. Do not suppress that failure or publish the partial release.

After the release evidence has been reviewed and the local stack is available,
publish and provision that exact revision, then enable the VM workflow:

```sh
cargo dev platform-images publish --revision <same-commit-sha>
cargo dev repository-images enable --revision <same-commit-sha>
cargo dev repository-images status
```

`enable` is idempotent for identical immutable inputs and refuses to replace a
different enabled release. A normal daemon restart then consumes the installed
roots; it does not repeat platform-image work.

To turn the workflow off without removing reviewed release/catalog evidence:

```sh
cargo dev repository-images disable
```

To remove only generated workflow state after disabling it:

```sh
cargo dev repository-images clean
```

## Repository contract

An image definition lives at the repository root in `heph.images.toml`. Its
Dockerfile and context are selected from the exact received Git commit. The
Dockerfile begins with `FROM heph-base`; `heph-base` is a reviewed,
digest-pinned execution base selected by the declaration. Other base images,
mutable tags, remote `ADD`/`COPY`, package-manager downloads, symlinked
contexts, sockets, credentials, and networked builds are rejected.

For a Hugo site, commit the checked and checksum-pinned Hugo archive or binary
with the site source, then copy it in the offline Dockerfile. The build VM has
no network, host socket, registry token, tenant secret, or reusable state. A
fresh 8 GiB sparse ext4 scratch disk is attached only to that builder VM for
Buildah's ownership-changing temporary storage; it is neither a host mount nor
reused by another preparation. The source and approved base remain read-only, while the
candidate layout is separately sealed before verification. A
distinct verifier VM receives only the candidate OCI layout read-only and
produces bounded SBOM, scan, and rootfs outputs. Its trusted command copies
the pinned offline Trivy database into a fresh job-scoped cache before
scanning, because analysis cache writes must never alter the read-only verifier
root. It moves the verified rootfs out of Umoci's temporary bundle and removes
only the remaining job-owned bundle metadata. Its one-shot guest process
inherits the daemon's reviewed 8,192-descriptor capacity for the bounded
Ubuntu-rootfs export. The trusted bootstrap applies that capacity only while
launching the exact platform verifier command as guest root; it does not
change host or tenant-agent limits. Only after that verifier succeeds may the
host-controlled publisher receive its short-lived exact registry credential.

The verifier rejects every HIGH or CRITICAL finding with an available upstream
fix. Findings without a fix remain recorded in the scan evidence; the reviewed
platform base is updated when a fix becomes available.

Once materialized, the immutable output can be selected by a normal build or
release contract. A failed, unverified, or unmaterialized image remains
unselectable. Use **Build** or **Rebuild** on the project image resource; its
history records safe requested, preparing, published, materializing, ready, or
failed transitions.
