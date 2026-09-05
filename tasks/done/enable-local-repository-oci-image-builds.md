# Enable local repository-owned OCI image builds

Owner: Codex

## Outcome

A local Hephaestus installation started with `cargo dev run` can prepare and
use an approved repository-owned OCI image through the same durable,
isolated image lifecycle as production. Repository OCI images are
first-class project resources, rather than opaque generic runs: a project
such as `cooking-blog` can commit a Hugo image definition and offline
Dockerfile input, inspect its source and preparation history, request a
build or rebuild, and select the resulting immutable image for a build or
release without hand-configuring daemon paths, using a host container socket,
or allowing arbitrary image pulls.

## Acceptance status

The local supervisor now wires the explicitly enabled workflow to separate
networkless builder and verifier microVMs. It does not invoke Podman or
Buildah itself. The reviewed local release provides all four execution bases
and the two platform-operation VM images; a project image is materialized only
after independent verification, publication read-back, and daemon-owned root
manifest installation.

The legacy `scripts/test-repository-oci-builder-e2e.sh` still exercises the
predecessor builder model and is not the acceptance harness for this workflow.
The hardware-gated local acceptance below was performed against `cargo dev`
with the actual libkrun VM boundary.

## Locked decisions

| Area | Decision |
| --- | --- |
| Local lifecycle | `cargo dev` remains the only local supervisor. It must not silently build or publish platform images or project images at startup. |
| Explicit enablement | A developer explicitly installs reviewed local platform images and explicitly enables the repository-image VM workflow before a project definition can be prepared. Status clearly distinguishes unavailable prerequisites from a failed project build. |
| Project UX | A repository OCI image is a first-class resource owned by its project and source repository. Users request **Build** or **Rebuild** from that resource and inspect its durable phase history; the underlying builder/verifier executions remain audit records, not generic user-facing runs. |
| Image source | A repository defines images only in root-level `heph.images.toml`; its Dockerfile and context are read from the exact pushed Git commit. It can select only an approved, digest-pinned platform base. |
| Dockerfile boundary | The first stage uses the reserved `heph-base`; other base references, remote `ADD`/`COPY`, host paths, sockets, ambient credentials, and mutable registry tags remain rejected. |
| Network and dependencies | Repository image builds remain network-disabled. A Hugo image must copy a reviewed, checksum-pinned Hugo binary/archive that is committed in the project source (or use a future approved Hugo platform base); no `apt`, `curl`, or package-manager download is permitted in the build. |
| Builder VM | A one-shot image-builder microVM receives an exact source checkout and approved base OCI layout read-only, plus a fresh private writable output mount. It has no network, host socket, registry credential, tenant secret, or reusable state. The repository Dockerfile may compromise this VM but cannot escape it. |
| Standard images | The platform supplies and explicitly provisions distinct minimal-Ubuntu, digest-pinned `oci-builder-ubuntu` and `oci-verifier-ubuntu` VM images. They contain only their pinned operational tools, are administrator-owned, and are never project image bases or ordinary agent-contract selections. |
| Verifier VM | A separate one-shot verifier microVM uses `oci-verifier-ubuntu` and receives the candidate OCI layout read-only. Its pinned tooling creates the SBOM and offline scan evidence and verifies descriptor/content shape. No output from the untrusted builder is approved merely because it claims a scan passed. |
| Publication | Only after verifier success may the host-controlled publisher receive a short-lived exact pull/push registry token. The image-builder and verifier VMs never receive a registry credential. |
| Materialization | A successfully verified and published image is materialized into the daemon's digest-to-rootfs manifest before it is selectable. Failed or non-materialized image revisions never run. |
| Observability | The UI and API expose source commit, approved base digest, preparation state, output digest, scan/provenance evidence, materialization state, and safe diagnostics—never tool credentials, registry tokens, or filesystem paths. |

## Dependencies

- [`provision-local-platform-builder-images.md`](provision-local-platform-builder-images.md)
- [`../done/repository-oci-builders.md`](../done/repository-oci-builders.md)
- [`expose-agent-release-build-journey.md`](expose-agent-release-build-journey.md)
- [`../todo/mvp-05-golden-cooking-agent-journey.md`](../todo/mvp-05-golden-cooking-agent-journey.md)

## Non-goals

- General Docker/Podman access from a guest, build, or repository.
- Pull-through registries, arbitrary registry references, mutable tags, or
  networked Dockerfile package installation.
- Automatically installing large platform images during `cargo dev run`.
- General CI pipelines or application deployment/serving; this task prepares
  OCI images for the existing release lifecycle only.

## Implementation shape

### Product model

- Repository OCI images are first-class project resources, owned by a project
  and sourced from one repository's exact Git revision.
- The UI and API expose the image definition, source commit, approved base,
  immutable output digest, verifier evidence, materialization state, and safe
  diagnostics.
- Project images have a dedicated **Images** tab on the project page. It lists
  only resources the viewer is authorized to inspect and provides the
  authorized Build/Rebuild and detail/history surfaces.
- Build/Rebuild creates a durable image-preparation execution visible in the
  image's history; it is not presented as an arbitrary generic run.
- Ready images are selectable in existing agent/release build contracts;
  `platform_operation` images are never tenant-selectable.

### Trusted workflow

- Keep the catalog role split between ordinary execution images and
  `platform_operation` images.
- Explicitly provision six reviewed local platform images through
  `cargo dev platform-images`: the four execution bases plus minimal-Ubuntu
  `oci-builder-ubuntu` and `oci-verifier-ubuntu`.
- Add `cargo dev repository-images status|enable|disable|clean`.
  `cargo dev run` validates and starts an already-enabled workflow only; it
  never builds or publishes images itself.
- Each preparation follows this exact sequence:
  1. Materialize the exact Git checkout and approved base layout into private
     roots.
  2. Launch a fresh networkless builder microVM with source/base read-only and
     candidate layout writable. It has no token, secret, host socket, or
     reusable state.
  3. Host-seal the candidate, then launch a separate verifier microVM with the
     candidate read-only and bounded evidence/rootfs output writable.
  4. Validate verifier output, then issue the host-controlled publisher a
     short-lived exact registry token for publication and read-back only.
  5. Atomically install the verified rootfs and mark the project image ready.
- Builder/verifier use minimal Ubuntu images with only required pinned tooling.
  The daemon never invokes Podman or Buildah directly.

### Durability and safety

- Durable image-preparation records bind immutable inputs and phase results for
  checkout, build, verification, publication, and materialization.
- Recovery uses sealed durable inputs only. Expired/crashed attempts retry
  safely or become actionable failed states; they never reuse mutable
  checkout, candidate, evidence, or credential paths.
- Dockerfiles remain offline: `FROM heph-base`, approved digest-pinned base
  only, and no remote source/download, traversal, symlink, socket, credential,
  or arbitrary registry reference.
- Keep private roots mode-restricted and non-overlapping. Redact paths, tokens,
  secrets, and raw tool output from events, logs, metrics, and database/UI
  payloads.

### Acceptance and documentation

- Update platform-image release/catalog tooling for six images and correct the
  catalog-generator path drift.
- Add the committed Hugo fixture and a `cooking-blog` tutorial using vendored,
  checksum-pinned Hugo input.
- Cover roles, authorization, unavailable prerequisites, recovery, malformed
  or tampered layouts, remote-source rejection, verifier failure, publication
  read-back, materialization, and no-leak behavior.
- Add a hardware-gated local E2E covering explicit install/enable, pushed Hugo
  definition, separate builder/verifier VMs, approved publication/
  materialization, and a Hugo build using that image.
- Finish with workspace formatting, Clippy, tests, docs, `cargo dev quality`,
  and `git diff --check`.

## Implementation checklist

- [x] **1. Define the local operator journey**
  - [x] Specify the explicit `cargo dev` commands for inspecting local
    prerequisites, installing reviewed platform bases, enabling/disabling the
    repository-image worker, and scoped cleanup.
  - [x] Make every command idempotent or fail safely when immutable evidence
    conflicts; never overwrite a release, catalog record, or materialized
    rootfs.
  - [x] Make `cargo dev status` identify missing platform bases, absent tool
    installation, missing base layouts, and disabled worker state separately.
  - [x] Keep normal startup side-effect free: it may validate existing state
    but cannot build, scan, publish, or register images.
  - [x] Expose project image resources and their safe phase histories through
    the existing API and an **Images** project-page tab; provide authorized
    Build/Rebuild actions without presenting OCI preparation as a free-form
    generic run.

- [x] **2. Implement the image-builder and verifier VM workflow**
  - [x] Define durable builder, verifier, and publication job identities and
    state transitions; each phase binds exact input digests and never trusts a
    mutable path or prior process state.
  - [x] Create private, mode-restricted local roots for builder checkouts,
    candidate OCI layouts, verifier evidence, exported rootfs, and temporary
    publication credentials without overlapping repository, VM, secret, or
    artifact roots.
  - [x] Generate and atomically validate the approved digest-to-OCI-layout
    manifest from explicitly installed platform images.
  - [x] Add a pinned, administrator-owned image-builder VM root containing
    Buildah and only the tools needed to create an OCI layout. This is a
    minimal Ubuntu operational image, not a project base; it must run
    Dockerfiles with network disabled and the approved `heph-base` layout.
  - [x] Add a distinct minimal Ubuntu verifier VM root containing offline
    Trivy, Syft, Umoci, and descriptor validation tooling. It must receive
    the candidate layout read-only and emit bounded evidence into a fresh
    host-validated output mount.
  - [x] Wire `scripts/run-local.sh` and the `cargo dev` supervisor to start
    the VM workflow only after explicit enablement and validated local roots.
    The daemon must retain its current user-namespace and libkrun boundary;
    it must never invoke Podman or Buildah directly.
  - [x] Ensure crash/restart recovery resumes or honestly fails each durable
    phase without reusing a mutable checkout, candidate layout, verifier
    output, or temporary credential.
  - [x] Prove an unconfigured local daemon reports a safe unavailable state
    rather than attempting a partial preparation.

- [x] **3. Complete platform-base installation for local use**
  - [x] Make the explicit platform-image build/publish operation record the
    approved `ubuntu-native`, `rust-ubuntu`, `typescript-node-ubuntu`, and
    `python-ubuntu` catalog entries and their local OCI layouts.
  - [x] Define, build, scan, attest, publish, and materialize the distinct
    platform-owned `oci-builder-ubuntu` and `oci-verifier-ubuntu` VM images.
    Keep them unavailable as project Dockerfile bases and ordinary
    `agent.toml` image selections.
  - [x] Verify catalog availability, registry read-back, provenance, scan,
    and rootfs materialization after publication.
  - [x] Prove a `cargo dev` restart consumes installed bases without repeating
    image work.

- [x] **4. Exercise a project-owned Hugo image**
  - [x] Add a small committed fixture repository containing a valid
    `heph.images.toml`, an approved-base `Dockerfile` beginning with
    `FROM heph-base`, and a checksum-pinned, vendored Hugo input.
  - [x] Prove the image-builder VM runs from the exact received Git commit
    with network disabled, rejects a Dockerfile download attempt, and records
    the exact base/output digests.
  - [x] Prove the separate verifier VM rejects a malformed or tampered OCI
    output and records scan/SBOM evidence only for a valid candidate layout.
  - [x] Select the prepared image in an `agent.toml` build contract and prove
    an isolated build executes `hugo`, imports only declared artifacts, and
    preserves image provenance in the resulting draft release.
  - [x] Prove ordinary local developer use can follow the same steps with the
    `cooking-blog` repository, without copying fixture paths or credentials.

- [x] **5. Preserve authorization and failure semantics**
  - [x] Verify project members can inspect only their own image definitions,
    revisions, Build/Rebuild controls, and redacted preparation histories;
    outsiders and anonymous callers are denied.
  - [x] Verify invalid keys, unavailable bases, arbitrary base references,
    symlinked contexts, remote sources, builder compromise/failure, verifier
    failure, scan failure, missing materialization, and worker restart/crash
    paths have durable, actionable safe states.
  - [x] Verify no registry token, build-tool credential, vendored secret,
    source-private value, or absolute private path appears in PostgreSQL, NATS,
    logs, events, metrics, UI payloads, or process arguments.

- [x] **6. Document and verify**
  - [x] Add a concise local tutorial for building a project-owned offline
    Hugo image; explain the project image resource, its Build/Rebuild history,
    and the distinct build image versus runtime image selection.
  - [x] Document the explicit heavy platform-image installation step, expected
    disk/time cost, status inspection, and narrowly scoped cleanup.
  - [x] Add focused Rust, CLI, PostgreSQL, VM-boundary, worker, and browser
    tests for the local operator journey and Hugo fixture.
  - [x] Run `cargo fmt --all -- --check`.
  - [x] Run `cargo clippy --workspace --all-targets --all-features`.
  - [x] Run `cargo test --workspace --all-features`.
  - [x] Run `cargo doc --workspace --all-features --no-deps`.
  - [x] Run `cargo dev quality`.
  - [x] Run `git diff --check`.

## Completion evidence

- [x] Record the explicit platform-base installation revision and catalog
  digests used by a fresh local daemon.
- [x] Record a successful local project-owned Hugo image-builder VM and
  independent verifier VM run from a pushed commit, its
  output digest/provenance/scan evidence, and a release build using that exact
  image.
- [x] Record denied/failed evidence for one unapproved Dockerfile base and
  one attempted network or remote Dockerfile source.

## Recorded local acceptance (2026-09-01)

- Explicit local platform release and enabled workflow revision:
  `581b939d5ad5e5a81e77ad01ad8931487a8d2bcf`.
  The builder and verifier were the reviewed platform-operation references
  `oci-builder-ubuntu@sha256:05494c04f4265792e17de9304517bf098864777c0f5610ba36c77ab5b8c7e376`
  and
  `oci-verifier-ubuntu@sha256:1e1b9618333711c130598a55956c9051b898fb100a4270d584340aa29c4ef09c`;
  `repository-images status` reported the workflow enabled and usable.
- The pushed `cooking-blog` source revision
  `326edf3d585359a724913d35ac228d4dff6c6ee2` produced project image
  `1fff4c68-881f-4e9f-8295-256c061b897c`, published and read back as
  `sha256:a860f424f4738ff1b930a32736d4991e6baf109055ff8c504c037586524df0ee`.
  Its source base was the approved `ubuntu-native` digest
  `sha256:9de0d1815207e34a0bb9ec025390590b1e5ebcef01fe83ba9024d768ddd38754`.
  Durable provenance recorded context digest
  `sha256:1403c0517f6363637eb85f26413cda1c16e1889f201f3ab25a98fec7c3abab79`,
  plus separate SBOM and attestation references.
- A subsequent pushed release source revision
  `341b57f5136cc067a2983503b9d098a7eb4187b6` selected that exact ready project
  image with `[build] image = { project_image = "cooking-blog-hugo" }`.
  The isolated, network-disabled Hugo build completed as
  `f411930a-0659-4899-b09e-1899b148a1d8` and created draft release
  `b23c2699-9871-4130-ab41-b437358d995e`, containing only the declared
  `public/` artifacts.
- Focused worker tests reject unapproved bases and remote `ADD`/`COPY` inputs;
  the live restart exercise expired an in-flight preparation lease and yielded
  a durable, redacted actionable failure instead of reusing a stale checkout.
- On the post-fix daemon, `f5b0daf65f2d9b303c74fdceee3e8228f5d5cf2a`
  materialized as project image `7df47d2f-c1eb-4463-b268-3ceef473f107` with
  digest
  `sha256:6033728a4b63a2f47687c1690dee190536816708960df51096a69e42ca710375`.
  Without restarting the daemon, the next pushed source revision
  `34312899b6acbdb8110f055b5a50ae0a36a857d5` automatically completed Hugo
  build `10896c5f-0992-451e-ab15-232e05f97f97` and draft release
  `20c055ac-a72d-48c7-b3c1-d7210e808649` using that exact project image.
  This proves the trusted materialization cache refresh, separately from
  daemon startup, and the resulting release contained only declared
  `public/` artifacts.
