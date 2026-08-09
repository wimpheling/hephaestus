# MVP 01.2: Replace controlled result publication with runtime Git

Owner: unassigned

## Outcome

Replace the current universal runtime-output contract—write a detached working
tree and let a host importer create a result ref—with normal Git access backed
by the exact-run capabilities from MVP 01 and MVP 01.1. An agent can use a
normal Git worktree, remote, commit, and push only where its immutable instance
binding permits it.

The existing controlled result-ref importer remains an explicit proposal mode
for agents that should never mutate a canonical branch. It is no longer the
only agent output mechanism.

## Locked decisions

| Area | Decision |
| --- | --- |
| Source provenance | A release remains attributable to its source repository and build inputs, but source provenance is not a runtime repository grant, mount, remote, or ambient authority. |
| Repository authority | Trigger routing and repository resource authority are distinct. A repository push may start a run without granting the runtime Git access, and a Git resource grant need not be a trigger. |
| Guest contract | A bound repository is delivered as an ordinary Git worktree with a capability-scoped Hephaestus remote. The guest uses standard Git commands; it receives no human credential or canonical storage mount. |
| Publication | A permitted guest push uses the normal Git receive path and capability checks. Proposal/result refs remain host-owned controlled publication. |
| Trigger-safe writes | A runtime binding may require a fast-forward only from its exact triggering commit. The repository/release defines the meaning of that constraint; runtime-originated pushes are attributed and do not recursively trigger that attachment. |
| Network | The Hephaestus Git endpoint is an internal capability endpoint, not external Internet egress. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md`](mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md)

## Implementation checklist

- [ ] **1. Split trigger routing from repository capabilities**
  - [ ] Separate the current attachment's repository/ref trigger selection
    from named immutable repository resource bindings.
  - [ ] Preserve exact triggering commit, repository, ref, and actor
    provenance without inferring read or write authority from the trigger.
  - [ ] Migrate existing attachments to explicit proposal-mode behavior and
    retain historical runs and result refs.

- [ ] **2. Deliver Git-native runtime worktrees**
  - [ ] Materialize a normal worktree at the exact authorized commit and
    configure only the capability-scoped internal Hephaestus remote.
  - [ ] Keep canonical bare storage, human credentials, unrelated remotes, and
    host filesystem paths inaccessible to the guest.
  - [ ] Provide bounded bootstrap metadata naming each declared repository
    binding without exposing reusable bearer material in arguments, logs, or
    durable run records.

- [ ] **3. Publish through normal Git receive**
  - [ ] Route runtime pushes through the same authenticated Git HTTP receive
    path used by users and PATs.
  - [ ] Enforce the run's repository/ref/path/transition capability and use
    compare-and-swap/fast-forward checks against the exact allowed parent.
  - [ ] Attribute runtime-originated receives to the instance, revision, run,
    and capability binding; suppress only the explicitly configured recursive
    trigger path.
  - [ ] Define recovery for guest exit after accepted push, receive failure,
    duplicate retry, revocation during transfer, and result import failure.

- [ ] **4. Retain proposal mode deliberately**
  - [ ] Make controlled workspace import and host-created result refs an
    explicit release capability for review/proposal agents.
  - [ ] Prevent a proposal-mode runtime from acquiring a Git write remote.
  - [ ] Update release, instance, workspace, forge, runtime, authorization,
    and architecture documentation to describe both modes and their authority
    boundaries.

- [ ] **5. Verify migration and security**
  - [ ] Add real Git/runtime tests for allowed triggering-commit fast-forward, denied
    repository/ref/path/delete/force pushes, source-repository denial,
    expired and revoked capability denial, and no recursive self-trigger.
  - [ ] Preserve and update existing proposal-result golden coverage.
  - [ ] Run the repository quality gate and Git credential sentinel scans.

## Verify and document

- Document the two publication modes, trigger-versus-resource authority,
  capability-scoped remote bootstrap, receive attribution, and recursive
  trigger suppression.
- Run the real Git/runtime integration suite against the production receive
  path, with PostgreSQL-backed authorization where applicable. Cover migration
  of existing attachments and proposal-mode compatibility as well as the
  permitted and denied transitions listed above.
- Run `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features`,
  `cargo test --workspace --all-features`, and
  `cargo doc --workspace --all-features --no-deps`.
- Before repository handoff, run `git diff --check` and `cargo dev quality`.

## Completion evidence

The completed task records representative runtime Git transcripts for an
allowed push and each denied boundary; migration fixtures for existing
proposal-mode attachments; receive audit records with instance, revision, and
run attribution; and proof that capability material is absent from guest logs
and durable run records. It also records the verification commands, results,
and any explicitly justified test-environment exclusions.
