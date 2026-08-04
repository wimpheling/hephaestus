# Expose the agent release build journey

Owner: Codex

## Outcome

Expose the complete agent release-build journey in the Heph UI. A user should
be able to create the project and repository, push an agent, observe and
inspect its build, review and publish the resulting draft release, and import
the published agent without needing temporary JSON commands or direct database
operations.

This is an **agent release build system**, not general-purpose CI. The target
journey is:

```text
Repository push
→ agent.toml validation
→ isolated build
→ draft release
→ human publication
→ importable release agent
```

## Scope

- [x] Expose every operation required by the release-build journey through the
  product UI and the Connect RPCs behind it.
- [x] Keep validation, authorization, persistence, and resulting events in the
  Rust services; do not add Phoenix SQL or temporary JSON commands.
- [x] Use typed product-event streams for live UI updates.

## Dependencies

- [x] Complete the Connect/event refactoring plan required by the target
  architecture.
- [x] Confirm the project, repository, release, agent import, and Git smart
  HTTP operations needed by this journey are available through typed RPCs.
- [x] Confirm the UI has the authenticated organization and project navigation
  needed to enter the journey.

## Locked product decisions

| Area | Decision |
| --- | --- |
| Product scope | Agent release builds rather than general-purpose CI. |
| Publication | Explicit human publication of a successful draft release. |
| Draft version | The publisher chooses the release version while the release is still a draft. |
| Build actions | Retry an attempt, rebuild immutable inputs for verification, and build another commit are distinct actions in both the API and UI. |
| Builder images | Platform-curated and digest-pinned catalog, with project-owned OCI builders based on approved platform images. |
| Dependencies | Begin with vendored/offline dependencies and curated toolchain images. |
| Fixtures | Exercise the real application workflow instead of seeding final releases directly. |

## Non-goals

This task does not introduce arbitrary CI jobs, matrices, test reports, build
caches, user-provided secrets, service containers, deployment workflows, or
pipeline composition. Controlled dependency caches may be added later under an
explicit platform policy.

## Implementation checklist

- [x] **1. Create a project in the UI**
  - [x] Add the organization Projects page entry point.
  - [x] Add the create-project form for name and description.
  - [x] Add validation, authorization, persistence, and resulting event
    handling to `ProjectService.CreateProject`.
  - [x] Navigate to the project overview after successful creation.
  - [x] Show useful validation and authorization errors without bypassing the
    service boundary.
  - [x] Add browser coverage for successful creation, invalid input, and
    denied access.

- [x] **2. Create and inspect a repository in the UI**
  - [x] Add the project Repositories page entry point.
  - [x] Add the create-repository form for name, visibility, and default
    branch.
  - [x] Navigate to the empty repository page after successful creation.
  - [x] Show the Git remote URL and authentication instructions.
  - [x] Show initial push commands for the selected default branch.
  - [x] Explain the expected location and role of `agent.toml`.
  - [x] Add repository routes:
    - [x] `/projects/:project_id/repositories/new`
    - [x] `/repositories/:repository_id`
  - [x] Add browser coverage for repository creation and the empty-repository
    push instructions.

- [x] **3. Expose repository files, commits, branches, builds, releases, and
      agents**
  - [x] Add repository navigation for:
    - [x] Files.
    - [x] Commits.
    - [x] Branches.
    - [x] Builds.
    - [x] Releases.
    - [x] Agents.
  - [x] Ensure each tab has an authorized, typed data source and a useful
    empty/loading/error state.
  - [x] Ensure live updates cannot reveal data from another project or
    repository.

- [x] **4. Trigger and list builds**
  - [x] Validate version-2 `agent.toml` during the push workflow.
  - [x] Automatically request a build for a matching push.
  - [x] Add a Builds tab at `/repositories/:repository_id/builds`.
  - [x] Show build state, source commit, source ref, trigger, agent key,
    builder image, start time, duration, artifact count, and draft/published
    release result in each build row.
  - [x] Support a deliberate manual build request through the UI where the
    product policy permits it.
  - [x] Show clear states for queued, running, succeeded, failed, cancelled,
    and unavailable builds.
  - [x] Update the list through `WatchRepositoryBuilds`.
  - [x] Preserve cursor/reconnect behavior and suppress duplicate visible
    transitions.
  - [x] Add the required RPC and UI coverage for automatic push builds and
    manual build requests.

- [x] **5. Inspect a build**
  - [x] Add the build detail route
    `/repositories/:repository_id/builds/:build_id`.
  - [x] Show the exact source commit and configuration hash.
  - [x] Show the parsed build declaration.
  - [x] Show builder image identity and digest.
  - [x] Show resource limits and network policy.
  - [x] Show the durable state timeline.
  - [x] Show bounded stdout/stderr logs with reconnect and truncation states.
  - [x] Show exit status and actionable diagnostics.
  - [x] Show declared artifacts versus produced artifacts.
  - [x] Show the imported artifact manifest.
  - [x] Link to the resulting draft release.
  - [x] Expose retry or rebuild actions only when the build state and
    authorization allow them.
  - [x] Label the three build actions with their actual semantics:
    - [x] **Retry attempt** — retry the same build identity after an
      infrastructure or recoverable failure.
    - [x] **Rebuild for verification** — execute the same immutable inputs
      again and compare the resulting manifest.
    - [x] **Build another commit** — create or select a build for different
      source.
  - [x] Do not expose a generic `Rebuild` action that hides these distinctions.
  - [x] Add `BuildService.GetBuild`, `BuildService.RequestBuild`,
    `BuildService.RetryBuild`, `BuildService.RebuildForVerification`,
    `BuildService.WatchBuild`, and `BuildService.StreamBuildLogs` as required
    by the UI.
  - [x] Add browser coverage for successful, failed, retried, and verification
    rebuild request journeys.
    - The browser harness creates a successful build through the real worker,
      seeds a failed build for retry, and verifies that retry and immutable
      verification requests commit their respective durable outbox commands.
      Completed verification mismatches show the durable expected and actual
      manifests on the build page.

- [x] **6. Review and publish a draft release**
  - [x] Add the release page and the Releases repository navigation.
  - [x] Show the draft review flow:
    - [x] Review the draft.
    - [x] Choose a release version.
    - [x] Inspect artifacts and the exported agent contract.
    - [x] Publish explicitly.
  - [x] Allow versions such as `v1.0.0`, `2026.07`, and
    `experimental-4` when they satisfy the release-version policy.
  - [x] Keep publication explicit by default and require the appropriate
    authorization.
  - [x] Freeze these values at publication:
    - [x] Version.
    - [x] Source commit.
    - [x] Build identity.
    - [x] Configuration.
    - [x] Artifact manifest.
    - [x] Exported agent contract.
    - [x] Parameter and secret-slot schemas.
    - [x] Runtime policy ceilings.
  - [x] Show published state and prevent mutation of frozen release data.
  - [x] Add `ReleaseService.GetRelease`, `ReleaseService.SetDraftVersion`,
    `ReleaseService.PublishRelease`, and `ReleaseService.WatchRelease` as
    required by the UI.
  - [x] Add browser coverage for draft review, invalid versions, denied
    publication, successful publication, and already-published releases.

- [x] **7. Import the published agent through the UI**
  - [x] Show the published agent in the authorized project and repository
    views.
  - [x] Add the import-agent action for an authorized project.
  - [x] Show the selected immutable release and exported agent contract before
    confirmation.
  - [x] Show parameter, secret-slot, and runtime-policy requirements without
    exposing secret values.
  - [x] Confirm the imported agent is linked to the published release and not
    to mutable source or a draft build.
  - [x] Add browser coverage from publication through import and initial agent
    inspection.

- [ ] **8. Provide the builder-image catalog**
  - [x] Replace the daemon environment-variable map with a platform-owned
    catalog exposed through the appropriate UI and service boundary.
  - [x] Configure workers with a versioned, explicit digest-to-rootfs manifest;
    validate every reference, materialization path, and filesystem kind before
    starting the daemon.
  - [x] Keep the legacy single-root environment pair available only for the
    fixture VM backend so existing deterministic fixtures remain usable without
    weakening production configuration.
  - [x] Show each builder image's stable ID and display name.
  - [x] Show its immutable digest-pinned reference.
  - [x] Show toolchains and versions.
  - [x] Show supported architecture.
  - [x] Show preparation status.
  - [x] Show provenance and optional signature/SBOM.
  - [x] Show availability and retirement state.
  - [x] Show the permitted build-network ceiling.
  - [x] Resolve `agent.toml` builder selection through a catalog identity or
    immutable reference.
  - [x] Reject arbitrary unapproved image pulls and execution.
  - [ ] Add the initial catalog entries through a reviewed provisioning process:
    - [x] Define and attest the initial platform images and digest-manifest
      generator in the manually dispatched release workflow.
    - [ ] Register the reviewed artifact records for `ubuntu-native`, `rust-ubuntu`,
      `typescript-node-ubuntu`, and `python-ubuntu`.
    - [x] Define Ubuntu minimal for shell/native builds.
    - [x] Define the Rust builder on Ubuntu with pinned Rust and Cargo toolchains.
    - [x] Define the TypeScript/Node builder on Ubuntu with pinned Node, package manager,
      TypeScript, and bundler versions.
    - [x] Define the Python builder on Ubuntu with pinned CPython and package tooling.
  - [x] Use Ubuntu-based images as the initial compatibility-oriented default;
    reserve Alpine/musl images for a later explicit target rather than making
    them the universal builder.
  - [x] Provision platform defaults through the active bootstrap operator using
    a reviewed, explicit OCI manifest; the command is transactional, preserves
    stable catalog IDs, supports dry-run validation, and does not run at
    application startup or in a schema migration. See
    `docs/builder-catalog-provisioning.md`.
  - [x] Allow a project to define a custom builder from a committed Dockerfile
    and OCI build configuration.
    - [x] Define repository-owned Dockerfile discovery, approved-base, selector,
      lifecycle, and isolation rules in `tasks/in-progress/repository-oci-builders.md`.
    - [x] Restrict custom-builder base images to approved digest-pinned
      platform builders.
    - [x] Build custom OCI images in an isolated image-builder job with
      rootless Buildah, disabled network, no ambient credentials, approved
      `heph-base` OCI layouts, exact-Git checkouts, and an offline Trivy gate.
      - The optional single-node daemon worker is configured with explicit
        private roots and trusted absolute binary paths; see
        `tasks/in-progress/repository-oci-builders.md`.
    - [x] Record the resulting immutable OCI digest, provenance, scan result,
      and preparation state under the owning project.
    - [x] Materialize only prepared custom digests as VM builder roots.
      - The daemon updates its digest-to-rootfs manifest atomically and resolves
        build requests only from a successful local materialization row.
  - [x] Let `agent.toml` select either a platform builder key or an immutable
    project-owned builder identity; persist the resolved digest on each build.
  - [x] Keep builder-root selection separate from the agent runtime image
    contract.
  - [x] Keep network access disabled by default.
  - [x] Define and expose the dependency policy:
    - [x] Vendored/offline dependencies.
    - [x] Read-only platform dependency caches.
    - [x] Constrained package-registry egress.
    - [x] No ambient host credentials.
  - [x] Start with vendored dependencies plus curated toolchain images before
    adding controlled dependency caches.

- [x] **9. Replace direct fixture release seeding**
  - [x] Create the fixture project through application operations.
  - [x] Create the fixture repository through application operations.
  - [x] Commit real source plus `agent.toml`.
  - [x] Push through Git smart HTTP.
  - [x] Observe the real build request and worker.
  - [x] Wait for the draft release through the real build/release state machine.
  - [x] Publish through the release operation.
  - [x] Import the resulting agent through the browser journey.
  - [x] Permit a fast fixture VM backend in automated browser tests only when
    it exercises the same build and publication state machine.
  - [x] Remove fixture paths that insert final published releases directly.

- [x] **10. Prove authorization, durability, and product-event behavior**
  - [x] Verify project, repository, build, release, builder-catalog, and agent
    import actions enforce the owning organization/project authorization.
    - The browser journey proves authenticated catalog access, project-member
      repository-builder visibility, outsider denial, and anonymous redirect;
      PostgreSQL authorization integration coverage verifies the owning
      organization/project perimeter for durable resource operations.
  - [x] Verify unauthorized users cannot inspect source, logs, artifacts,
    releases, builder metadata, or live updates.
  - [x] Verify authoritative mutations and their product events commit
    durably together.
  - [x] Verify live build and release updates resume from a committed cursor
    after browser, daemon, or channel restart.
  - [x] Verify duplicate events do not create duplicate rows, transitions,
    release publication, or import side effects.
  - [x] Verify sensitive request values, secret values, and private build
    output are not exposed in logs, events, diagnostics, metrics, or browser
    payloads.

- [x] **11. Verify and document the complete UI journey**
  - [x] Add an end-to-end browser journey covering project creation,
    repository creation, push, build observation, build inspection, draft
    review, publication, and agent import.
  - [x] Cover empty, loading, success, failure, retry, reconnect, denied, and
    stale-resource states for each new page.
    - Browser scenarios cover failed/retry, verification request, reconnect,
      denial, and a completed verification mismatch with both immutable
      manifests visible.
  - [x] Document the supported `agent.toml` build declaration and builder
    selection contract.
  - [x] Document the distinction between retry, verification rebuild, and a
    build for another commit.
  - [x] Document the dependency, network, image provenance, and publication
    policies.
  - [x] Run `cargo dev quality`.
- [x] Run `git diff --check`.

## Current implementation boundary

The repository-owned builder contract is specified in
`tasks/in-progress/repository-oci-builders.md`. The current implementation has
the following durable boundary:

- `agent.toml` can select a platform key (`build.builder.kind = "platform"`)
  a legacy project-builder UUID (`kind = "project"`), or a repository-local
  key (`kind = "repository"`). Legacy digest-pinned `build.root_image` remains
  supported.
- `heph.builders.toml` is read from the exact received commit; safe Dockerfile
  and context paths are persisted as immutable repository-builder revisions.
- Receive and manual-build request paths resolve either selector transactionally
  against approved, ready catalog records and persist the immutable resolved
  digest in `build_requests.builder_image_reference`.
- Claimed worker input replaces the selector with that resolved digest, so VM
  execution remains digest-only and separate from the agent runtime image.
- Project Dockerfile/OCI definitions have durable lifecycle state, project
  authorization, RLS-backed persistence, product-event outbox publication, and
  an authorized read-only project-builders UI route. Definitions are discovered
  from committed repository configuration; callers cannot create or complete
  an arbitrary OCI digest through an RPC.
- The OCI preparation queue, exact checkout, rootless Buildah build,
  offline-scan/provenance output, Umoci rootfs export, durable result writes,
  root manifest generation, and supervised optional daemon worker are wired.
  Repository builder revisions enter `preparing` transactionally on receive.
- Local libkrun fixtures now use a pinned Ubuntu root image; Fedora remains a
  host/documentation concern rather than the product default.

Still deliberately open:

- Provision reviewed platform OCI artifacts and their local base layouts before
  enabling repository OCI workers; no manual completion RPC may be treated as
  execution of an arbitrary caller-provided digest.
- The four initial platform catalog rows require the reviewed workflow artifact
  to be applied by an operator. The repository contains the Ubuntu build,
  scan, attestation, and digest-manifest release path but no fake digests.

## Required UI and service surface

- [x] `ProjectService.CreateProject`
- [x] `BuildService.ListBuilds`
- [x] `BuildService.GetBuild`
- [x] `BuildService.RequestBuild`
- [x] `BuildService.RetryBuild`
- [x] `BuildService.RebuildForVerification`
- [x] `BuildService.WatchBuild`
- [x] `BuildService.StreamBuildLogs`
- [x] `ReleaseService.GetRelease`
- [x] `ReleaseService.SetDraftVersion`
- [x] `ReleaseService.PublishRelease`
- [x] `ReleaseService.WatchRelease`
- [x] `WatchRepositoryBuilds`
- [x] Project creation route: `/organizations/:organization_id/projects/new`
- [x] Repository creation route: `/projects/:project_id/repositories/new`
- [x] Repository route: `/repositories/:repository_id`
- [x] Build list route: `/repositories/:repository_id/builds`
- [x] Build detail route: `/repositories/:repository_id/builds/:build_id`
- [x] Builder catalog route: `/builders`

## Completion evidence

- [x] Record the fixture organization, project, repository, build, draft
  release, published release, and imported-agent IDs.
  - Latest browser fixture run: organization `e765e6a7-ee0d-4727-97e3-fbdebeb4621c`,
    project `de9c0f21-98f6-484a-a89f-529f50c8b26f`, repository
    `56c11be9-35cb-40a1-ab63-c26743cd97ed`, build
    `02340b62-b189-47b4-953f-9c9e21cefb2f`, release
    `130b5144-abdf-4aec-bee2-ff987aba4440`, and imported agent
    `4c967726-c20a-8a30-ac1a-86b341c20354`.
- [x] Record the source commit, build identity, configuration hash, artifact
  manifest, builder-image digest, and release version.
  - The build detail projection now records and renders each of these values;
    ephemeral fixture evidence is written to `real-build-journey-ids.json`.
- [x] Record browser screenshots for the complete journey and each meaningful
  failure or authorization state without including secrets or private data.
- [x] Record the exact verification commands and their results.

  Verification passed in this worktree:

  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features`
  - `cargo test --workspace --all-features`
  - `cargo doc --workspace --all-features --no-deps`
  - `scripts/check-generated.sh`
  - `cargo dev quality` — passed after bringing the project-builders route,
    state module, page component, and lifecycle coverage into the repository
    UI architecture.
  - Phoenix container `mix format && mix test` — 188 tests, 0 failures
  - `HEPHAESTUS_PLAYWRIGHT_SKIP_BROWSER_INSTALL=1 scripts/run-ui-e2e.sh` —
    full browser suite passed (`10 passed`, exit 0), including the complete
    create-to-import journey and custom build/release watch paths.
  - `git diff --check`
