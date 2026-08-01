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

- [ ] Expose every operation required by the release-build journey through the
  product UI and the Connect RPCs behind it.
- **Blocked:** Project/repository creation, build listing/detail/request,
  typed build watch/log/retry contracts, draft publication, and the builder
  catalog surface are implemented. Durable retry/rebuild execution, the
  complete build metadata projection, and the full create-to-import browser
  journey still require storage/fixture work.
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
| Builder images | Platform-curated and digest-pinned catalog. |
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
  - [ ] Add browser coverage for successful creation, invalid input, and
    denied access.
    - **Blocked:** The Playwright runner exists and static checks pass, but
      the current journey does not yet create a project through the browser.

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
  - [ ] Add browser coverage for repository creation and the empty-repository
    push instructions.
    - **Blocked:** The Playwright runner exists, but the current test uses a
      seeded repository and does not yet cover repository creation in-browser.

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

- [ ] **4. Trigger and list builds**
  - [x] Validate version-2 `agent.toml` during the push workflow.
  - [x] Automatically request a build for a matching push.
  - [x] Add a Builds tab at `/repositories/:repository_id/builds`.
  - [ ] Show build state, source commit, source ref, trigger, agent key,
    builder image, start time, duration, artifact count, and draft/published
    release result in each build row.
    - **Blocked:** The current typed Build summary exposes state, source,
      artifact count, and release state, but not trigger, agent key, builder
      image, start time, or duration.
  - [x] Support a deliberate manual build request through the UI where the
    product policy permits it.
  - [x] Show clear states for queued, running, succeeded, failed, cancelled,
    and unavailable builds.
  - [ ] Update the list through `WatchRepositoryBuilds`.
    - **Blocked:** No `WatchRepositoryBuilds` RPC exists; the UI uses the
      authorized repository product-event watch, which is the current
      canonical live boundary.
  - [x] Preserve cursor/reconnect behavior and suppress duplicate visible
    transitions.
  - [ ] Add the required RPC and UI coverage for automatic push builds and
    manual build requests.
    - **Blocked:** Automatic push/build orchestration exists in Rust, and
      manual request has focused state/page contracts, but browser coverage
      and a complete trigger contract are not yet available.

- [ ] **5. Inspect a build**
  - [x] Add the build detail route
    `/repositories/:repository_id/builds/:build_id`.
  - [ ] Show the exact source commit and configuration hash.
    - **Blocked:** `GetBuild` now exposes the exact source commit, but its
      response does not expose `configuration_hash`.
  - [ ] Show the parsed build declaration.
    - **Blocked:** No typed parsed-declaration response exists.
  - [ ] Show builder image identity and digest.
    - **Blocked:** The catalog now exists, but `Build` does not yet persist or
      project the selected builder identity and digest.
  - [ ] Show resource limits and network policy.
    - **Blocked:** The Build response does not carry the declaration policy.
  - [ ] Show the durable state timeline.
    - **Blocked:** `GetBuild` exposes the current state only; no timeline RPC
      or projection exists.
  - [x] Show bounded stdout/stderr logs with reconnect and truncation states.
  - [x] Show exit status and actionable diagnostics.
  - [ ] Show declared artifacts versus produced artifacts.
    - **Blocked:** Only the artifact count is currently exposed on Build.
  - [ ] Show the imported artifact manifest.
    - **Blocked:** The imported manifest is available on Release, not on the
      Build detail response.
  - [ ] Link to the resulting draft release.
    - **Blocked:** The detail page can link when a release ID is present, but
      there is no draft-release relation/action contract for every build.
  - [ ] Expose retry or rebuild actions only when the build state and
    authorization allow them.
    - **Blocked:** Typed retry and verification-rebuild RPCs exist, but both
      return explicit precondition errors until durable attempt reset and
      manifest-comparison storage are added.
  - [ ] Label the three build actions with their actual semantics:
    - [x] **Retry attempt** — retry the same build identity after an
      infrastructure or recoverable failure.
    - [x] **Rebuild for verification** — execute the same immutable inputs
      again and compare the resulting manifest.
    - [x] **Build another commit** — create or select a build for different
      source.
  - [x] Do not expose a generic `Rebuild` action that hides these distinctions.
  - [ ] Add `BuildService.GetBuild`, `BuildService.RequestBuild`,
    `BuildService.RetryBuild`, `BuildService.RebuildForVerification`,
    `BuildService.WatchBuild`, and `BuildService.StreamBuildLogs` as required
    by the UI.
    - **Blocked:** `GetBuild`, `RequestBuild`, `RetryBuild`,
      `RebuildForVerification`, `WatchBuild`, and `StreamBuildLogs` now have
      generated and registered contracts. Retry/rebuild remain intentionally
      unavailable until their durable storage semantics are implemented.
  - [ ] Add browser coverage for successful, failed, retried, and verification
    rebuild journeys.
    - **Blocked:** Playwright exists, but no browser journey can exercise
      retry/rebuild while those mutations return typed preconditions.

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
  - [ ] Add `ReleaseService.GetRelease`, `ReleaseService.SetDraftVersion`,
    `ReleaseService.PublishRelease`, and `ReleaseService.WatchRelease` as
    required by the UI.
    - **Blocked:** GetRelease, SetDraftVersion, and PublishRelease are
      implemented. A dedicated WatchRelease RPC is not present; the UI uses
      the authorized repository event watch.
  - [ ] Add browser coverage for draft review, invalid versions, denied
    publication, successful publication, and already-published releases.
    - **Blocked:** Playwright exists, but draft publication coverage still
      uses the seeded release journey rather than the complete create/build
      path.

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
  - [ ] Add browser coverage from publication through import and initial agent
    inspection.
    - **Blocked:** Existing project-agent and instance component/state tests
      cover the contract; the browser runner does not yet cover publication
      through import from a newly-created project.

- [ ] **8. Provide the builder-image catalog**
  - **Blocked:** The digest-pinned catalog domain, PostgreSQL adapter, typed
    RPCs, `/builders` UI, and selection validation are implemented. Approved
    catalog rows are intentionally not seeded, and the build orchestrator
    still has a separate environment-derived root-image map to replace.
  - [ ] Replace the daemon environment-variable map with a platform-owned
    catalog exposed through the appropriate UI and service boundary.
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
  - [ ] Add the initial catalog entries:
    - [ ] Fedora minimal for shell/native fixture builds.
    - [ ] Rust builder with pinned Rust and Cargo toolchains.
    - [ ] Node builder with pinned Node and package manager.
  - [x] Keep network access disabled by default.
  - [x] Define and expose the dependency policy:
    - [x] Vendored/offline dependencies.
    - [x] Read-only platform dependency caches.
    - [x] Constrained package-registry egress.
    - [x] No ambient host credentials.
  - [x] Start with vendored dependencies plus curated toolchain images before
    adding controlled dependency caches.

- [ ] **9. Replace direct fixture release seeding**
  - **Blocked:** The runner now exists and direct final-release publication
    was removed, but builds, artifacts, and the draft release are still
    prepared directly because the runner has no build worker workflow.
  - [x] Create the fixture project through application operations.
  - [x] Create the fixture repository through application operations.
  - [ ] Commit real source plus `agent.toml`.
  - [ ] Push through Git smart HTTP.
  - [ ] Observe the real build request and worker.
  - [ ] Wait for the draft release through the real build/release state machine.
  - [x] Publish through the release operation.
  - [ ] Import the resulting agent through the browser journey.
  - [ ] Permit a fast fixture VM backend in automated browser tests only when
    it exercises the same build and publication state machine.
  - [ ] Remove fixture paths that insert final published releases directly.

- [x] **10. Prove authorization, durability, and product-event behavior**
  - [ ] Verify project, repository, build, release, builder-catalog, and agent
    import actions enforce the owning organization/project authorization.
    - **Blocked:** The catalog RPC authenticates the mediator and its catalog
      adapter is RLS-protected, but dedicated organization authorization and
      seeded approved-image integration tests remain to be added.
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

- [ ] **11. Verify and document the complete UI journey**
  - **Blocked:** The Playwright runner and screenshot-capable test project
    exist, and the seeded publication/build inspection slice is green, but
    the complete create-to-import workflow is not yet executable.
  - [ ] Add an end-to-end browser journey covering project creation,
    repository creation, push, build observation, build inspection, draft
    review, publication, and agent import.
  - [ ] Cover empty, loading, success, failure, retry, reconnect, denied, and
    stale-resource states for each new page.
    - **Blocked:** Browser-level retry coverage is unavailable while durable
      retry/rebuild mutations remain preconditioned.
  - [x] Document the supported `agent.toml` build declaration and builder
    selection contract.
  - [x] Document the distinction between retry, verification rebuild, and a
    build for another commit.
  - [x] Document the dependency, network, image provenance, and publication
    policies.
  - [x] Run `cargo dev quality`.
  - [x] Run `git diff --check`.

## Required UI and service surface

- [x] `ProjectService.CreateProject`
- [x] `BuildService.ListBuilds`
- [x] `BuildService.GetBuild`
- [x] `BuildService.RequestBuild`
- [x] `BuildService.RetryBuild`
  - **Blocked:** The typed RPC is registered and returns a precise precondition
    until durable build-attempt reset storage is added.
- [x] `BuildService.RebuildForVerification`
  - **Blocked:** The typed RPC is registered and returns a precise precondition
    until durable manifest-comparison storage is added.
- [x] `BuildService.WatchBuild`
  - **Blocked:** The authorized resumable stream is implemented; dedicated
    browser coverage remains open.
- [x] `BuildService.StreamBuildLogs`
  - **Blocked:** The authorized resumable bounded-log stream is implemented;
    dedicated browser coverage remains open.
- [x] `ReleaseService.GetRelease`
- [x] `ReleaseService.SetDraftVersion`
- [x] `ReleaseService.PublishRelease`
- [ ] `ReleaseService.WatchRelease`
  - **Blocked:** A dedicated release watch RPC is not present; repository-scoped
    product events remain the canonical live source.
- [ ] `WatchRepositoryBuilds`
  - **Blocked:** RPC is not present; the Builds tab uses repository-scoped
    product events.
- [x] Project creation route: `/organizations/:organization_id/projects/new`
- [x] Repository creation route: `/projects/:project_id/repositories/new`
- [x] Repository route: `/repositories/:repository_id`
- [x] Build list route: `/repositories/:repository_id/builds`
- [x] Build detail route: `/repositories/:repository_id/builds/:build_id`
- [x] Builder catalog route: `/builders`

## Completion evidence

- [ ] Record the fixture organization, project, repository, build, draft
  release, published release, and imported-agent IDs.
  - **Blocked:** The Playwright runner passes the seeded publication and build
    inspection slice (`6 passed`, exit 0), but the fixture workflow still lacks
    a build worker and therefore cannot produce stable create-to-import IDs.
- [ ] Record the source commit, build identity, configuration hash, artifact
  manifest, builder-image digest, and release version.
  - **Blocked:** No complete create-to-import journey produced stable IDs;
    Build still does not expose configuration hash or selected builder digest.
- [ ] Record browser screenshots for the complete journey and each meaningful
  failure or authorization state without including secrets or private data.
  - **Blocked:** Playwright is configured, but no complete journey reached the
    screenshot checkpoints; see `docs/agent-release-build-browser-verification.md`.
- [x] Record the exact verification commands and their results.

  Verification passed in this worktree:

  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features`
  - `cargo test --workspace --all-features`
  - `cargo doc --workspace --all-features --no-deps`
  - `scripts/check-generated.sh`
  - `cargo dev quality`
  - Phoenix container `mix format && mix test` — 187 tests, 0 failures
  - `HEPHAESTUS_PLAYWRIGHT_SKIP_BROWSER_INSTALL=1 scripts/run-ui-e2e.sh` —
    full current suite passed (`6 passed`, exit 0), including seeded draft
    publication and build history/detail/provenance; the complete
    create-to-import journey remains blocked by the missing browser build-worker
    workflow documented in `docs/agent-release-build-browser-verification.md`.
  - `git diff --check`
