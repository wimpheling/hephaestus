# Expose the agent release build journey

This should become a separate product task, dependent on the Connect/event
refactoring plan. We should design it directly against the target architecture
rather than add more Phoenix SQL or temporary JSON commands.

The first important decision: I would not position this as general-purpose CI
yet. It is an **agent release build system**.

```text
Repository push
→ agent.toml validation
→ isolated build
→ draft release
→ human publication
→ importable release agent
```

General-purpose CI would imply arbitrary jobs, matrices, test reports, caches,
secrets, service containers, deployment workflows, and pipeline composition.
We do not need that surface to make agent development complete.

## Proposed product journey

### 1. Create a project

From the organization Projects page:

```text
Projects
  → Create project
  → Name and description
  → Project overview
```

Route:

```text
/organizations/:organization_id/projects/new
```

The Rust `ProjectService.CreateProject` RPC owns validation, authorization,
persistence, and the resulting event.

### 2. Create a repository

From the project Repositories page:

```text
Repositories
  → Create repository
  → Name, visibility, default branch
  → Empty repository page with push instructions
```

Routes:

```text
/projects/:project_id/repositories/new
/repositories/:repository_id
```

The repository page should prominently show:

- Git remote URL.
- Authentication instructions.
- Initial push commands.
- Expected location and role of `agent.toml`.

### 3. Push an agent project

A matching push containing version-2 `agent.toml` automatically creates a
build.

The repository should gain a **Builds** tab:

```text
Files / Commits / Branches / Builds / Releases / Agents
```

Route:

```text
/repositories/:repository_id/builds
```

Each build row shows:

- State.
- Source commit and ref.
- Trigger: push or manual.
- Agent key.
- Builder image.
- Start time and duration.
- Artifact count.
- Draft/published release result.

Updates arrive through `WatchRepositoryBuilds`.

### 4. Inspect a build

Route:

```text
/repositories/:repository_id/builds/:build_id
```

The build detail page should contain:

- Exact source commit and configuration hash.
- Parsed build declaration.
- Builder image identity and digest.
- Resource and network policy.
- Durable state timeline.
- Bounded stdout/stderr stream.
- Exit status and diagnostics.
- Declared versus produced artifacts.
- Imported artifact manifest.
- Draft release link.
- Retry or rebuild actions when applicable.

The UI must clearly distinguish:

- **Retry attempt**: retry the same build identity after an infrastructure or
  recoverable failure.
- **Rebuild for verification**: intentionally execute the same immutable inputs
  again and compare the resulting manifest.
- **Build another commit**: create/select a build for different source.

A generic “Rebuild” button would otherwise hide materially different
semantics.

## Publication

A successful build produces a draft release. The release page then exposes:

```text
Review draft
→ choose release version
→ inspect artifacts and exported agent contract
→ Publish
```

Publication should remain explicit by default.

I recommend letting the publisher assign the version while the release is
still a draft:

```text
v1.0.0
2026.07
experimental-4
```

Publication then freezes:

- Version.
- Source commit.
- Build identity.
- Configuration.
- Artifact manifest.
- Exported agent contract.
- Parameter and secret-slot schemas.
- Runtime policy ceilings.

After publication, the exported agent appears in authorized projects and can
be imported as an instance.

Required RPCs:

```text
BuildService.ListBuilds
BuildService.GetBuild
BuildService.RequestBuild
BuildService.RetryBuild
BuildService.RebuildForVerification
BuildService.WatchBuild
BuildService.StreamBuildLogs

ReleaseService.GetRelease
ReleaseService.SetDraftVersion
ReleaseService.PublishRelease
ReleaseService.WatchRelease
```

## Builder-image catalog

The daemon’s environment-variable map should become a real platform-owned
catalog.

A builder image should record:

- Stable ID and display name.
- Immutable digest-pinned reference.
- Toolchains and versions.
- Supported architecture.
- Preparation status.
- Provenance and optional signature/SBOM.
- Availability and retirement state.
- Permitted build-network ceiling.

`agent.toml` should select a catalog identity or immutable reference that
resolves through the catalog. It must not cause the daemon to pull and execute
arbitrary unapproved images.

Initial catalog:

- Fedora minimal: shell/native fixture builds.
- Rust builder: pinned Rust and Cargo toolchain.
- Node builder: pinned Node and package manager.

Network should remain disabled by default. We need an explicit policy for
dependencies:

- Vendored/offline dependencies — most reproducible initial option.
- Read-only platform dependency caches.
- Constrained package-registry egress.
- Never ambient host credentials.

I recommend beginning with vendored dependencies plus curated toolchain images,
then adding controlled dependency caches. Calling the system general-purpose
CI before solving caching, registry policy, and reproducibility would set the
wrong expectation.

## Fixtures

The current fixture seeder should stop fabricating published releases directly.

The replacement fixture journey should:

1. Create the project and repository through application operations.
2. Commit real source plus `agent.toml`.
3. Push through Git smart HTTP.
4. Observe the real build request and worker.
5. Wait for the draft release.
6. Publish it through the release operation.
7. Import the resulting agent through the browser journey.

A fast fixture VM backend can still be used in automated browser tests, but it
should exercise the same build and publication state machine rather than insert
final rows.

## Decisions to lock before detailing this task

My recommendations are:

- Scope: agent release builds, not general-purpose CI.
- Publication: explicit human publication with version chosen on the draft.
- Build actions: distinguish retry from verification rebuild.
- Builder images: platform-curated, digest-pinned catalog.
- Initial dependency policy: vendored/offline dependencies.
- Fixture policy: exercise the real application workflow, never seed final
  releases directly.
- Architecture dependency: implement through Connect RPCs and typed event
  streams only.
