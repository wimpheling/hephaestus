# Remote Telegram-controlled development and smoke environment

Owner: unassigned

## Outcome

Provide a private, persistent x86-64 Fedora development host that can run the
real libkrun/KVM Hephaestus smoke stack and can be safely operated from a
phone. Telegram is a constrained command-and-task interface; Tailscale gives
the owner private browser access to the smoke UI and OIDC sign-in flow.

The environment must run the same real-KVM manual-smoke path as `cargo dev`,
retain state across supervised restarts, and make a stable private smoke URL
available from the owner's phone.

## Locked decisions

| Area | Decision |
| --- | --- |
| Compute | Use an x86-64 bare-metal host or a cloud VM with verified nested KVM. A VPS merely *hosted* on KVM is insufficient; the `forge` account must have read/write access to `/dev/kvm`. |
| Host baseline | Fedora/Linux, non-root `forge` service account, libkrun 1.x with libkrunfw and passt, Podman, Rust toolchain, and a delegated cgroup-v2 subtree with CPU, IO, memory, and PID controllers. |
| Network access | Keep PostgreSQL, NATS, daemon RPC/Git HTTP, and host administration private. Use Tailscale Serve for the owner-facing web and development OIDC endpoints. Do not expose the development stack through a public tunnel. |
| Smoke authentication | The browser OIDC issuer and redirect URI must use the private Tailscale HTTPS origins, not host loopback addresses. The local fixture remains development-only and is never internet-public. |
| Telegram authority | A bot may invoke an explicit allowlist of control actions and create task requests. It must not provide arbitrary shell execution, secret retrieval, direct database access, or unrestricted Git credential issuance. |
| Service ownership | Systemd supervises the persistent environment. The Telegram bot and all runtime services operate as the dedicated non-root `forge` account, except narrowly scoped one-time host provisioning. |
| Coding workflow | Telegram task submission creates an isolated worktree/branch and reports diffs and verification results. Any agent-driven code change requires explicit owner approval before merge or deployment. |

## Dependencies

- [`../in-progress/repository-oci-builders.md`](../in-progress/repository-oci-builders.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](mvp-03-event-ingress-and-caddy-routing.md) for any future public Telegram gateway; this environment's bot is host-local control infrastructure, not platform product ingress.

## Non-goals

This task does not make the development OIDC fixture production identity,
expose Hephaestus or Telegram control endpoints publicly, add generic remote
shell access through Telegram, make Telegram a replacement for a full IDE, or
select a production event-ingress implementation.

## Implementation checklist

- [ ] **1. Select and accept a suitable host**
  - [ ] Choose either an always-on local x86-64 machine or an EU bare-metal
    provider; record location, billing/renewal ownership, console/recovery
    access, storage capacity, and backup destination.
  - [ ] If selecting a cloud VM, obtain a written provider guarantee for nested
    KVM and verify it before configuring the environment.
  - [ ] Size the host for concurrent Rust/Phoenix builds, PostgreSQL, NATS,
    container images, and microVM disks; document the chosen CPU, RAM, NVMe,
    and capacity headroom.
  - [ ] Install a supported Fedora release on x86-64 and apply automatic
    security updates with a documented reboot policy.
  - [ ] Verify `test -r /dev/kvm -a -w /dev/kvm`, `cargo dev doctor`, and
    `scripts/run-libkrun-integration.sh` under the non-root service account.

- [ ] **2. Provision the host security and persistence boundary**
  - [ ] Create the dedicated `forge` account and grant only the KVM, container,
    filesystem, and delegated-cgroup permissions required by the documented
    libkrun host contract.
  - [ ] Install and configure libkrun, libkrunfw, passt, Podman, Rust, Node,
    and the project prerequisites at pinned or documented compatible versions.
  - [ ] Create a systemd service and dedicated writable roots for repository,
    runtime, volumes, workspaces, artifacts, logs, and secret-key material.
  - [ ] Keep secret runtime paths mode 0700 and stored key material readable
    only by `forge`; do not place credentials in repository files or Telegram
    messages.
  - [ ] Configure encrypted, tested backups for Git repositories, PostgreSQL,
    NATS state, project-local persistent state, and host configuration; define
    retention and a restore exercise.
  - [ ] Document host-console, SSH/Tailscale recovery, service restart, log,
    update, backup, and restore runbooks.

- [ ] **3. Make the supported smoke stack remotely reachable**
  - [ ] Add explicit development configuration for an externally reachable
    browser URL and OIDC issuer URL; preserve loopback defaults for ordinary
    local development.
  - [ ] Update the local OIDC fixture so its advertised issuer, discovery,
    authorization, token, userinfo, and callback validation use configured
    private HTTPS origins rather than hard-coded `127.0.0.1` URLs.
  - [ ] Configure Phoenix's OIDC issuer and redirect URI from those origins and
    preserve the existing local browser and E2E paths.
  - [ ] Install Tailscale on the host and owner phone; apply an ACL that permits
    only the owner to reach the development host and enables Tailscale SSH only
    where needed.
  - [ ] Configure Tailscale Serve to proxy the Phoenix UI and OIDC fixture from
    stable private HTTPS origins while keeping backend listeners bound to
    loopback.
  - [ ] Verify a phone can load the private smoke URL, complete OIDC login,
    exercise the UI, and observe a real libkrun/KVM run after a host reboot.
  - [ ] Add a bounded health/status endpoint or status command that reports
    service readiness without revealing tokens, paths, identifiers, or logs
    containing sensitive data.

- [ ] **4. Supervise the persistent development environment**
  - [ ] Run `cargo dev --watch` or an equivalent explicitly documented
    supervisor under systemd, retaining intended local state across restarts.
  - [ ] Add separate, bounded operations for start, stop, restart, status,
    daemon-log tail, web-log tail, focused smoke checks, and the repository
    quality gate.
  - [ ] Ensure an interrupted build leaves the last known-good smoke service
    available whenever the existing supervisor semantics permit it.
  - [ ] Add disk, memory, KVM/runtime, backup freshness, service readiness,
    and failed-build alerts delivered privately to the owner.
  - [ ] Document resource cleanup and recovery using `cargo dev state` without
    deleting persistent state by default.

- [ ] **5. Implement a constrained Telegram control bot**
  - [ ] Create a host-local bot service using outbound long polling unless a
    separately authenticated private webhook is required.
  - [ ] Authenticate an explicit Telegram owner chat ID; reject all other
    chats, group messages, forwarded commands, malformed updates, and replayed
    control requests with safe diagnostics.
  - [ ] Store the bot token using systemd credentials or another host-secret
    mechanism readable only by `forge`; redact it from logs and responses.
  - [ ] Implement allowlisted `/status`, `/logs`, `/restart`, `/smoke`,
    `/quality`, and `/open` commands that call fixed control helpers rather
    than interpolating user text into a shell command.
  - [ ] Require a confirmation token for disruptive, long-running, or
    state-mutating actions; include the exact target and expiration in the
    confirmation response.
  - [ ] Bound log output, redact configured sensitive patterns, apply command
    timeouts and per-command concurrency limits, and record an audit trail.
  - [ ] Send completion, failure, smoke URL, commit, and verification summaries
    to the owner without sending raw credentials or secrets.

- [ ] **6. Add an approval-gated remote coding workflow**
  - [ ] Define a `/task` command that records the requested work with a stable
    task ID and creates an isolated Git worktree and branch without touching
    the active smoke checkout.
  - [ ] Define the coding-agent invocation, time/resource limits, repository
    write scope, test selection, and recovery handling; do not grant the agent
    deployment, secret, or Telegram-administration authority.
  - [ ] Return compact change summaries, commit IDs, test outcomes, and a
    reviewable diff or pull-request link to Telegram.
  - [ ] Implement explicit `/approve` and `/reject` transitions; approval may
    merge or update the smoke checkout only after stated verification succeeds.
  - [ ] Add tests proving tasks cannot alter another worktree, invoke arbitrary
    host commands, read service credentials, or deploy without owner approval.

- [ ] **7. Verify and document**
  - [ ] Add focused tests for remote-origin configuration, OIDC callback
    validation, Telegram authorization, command allowlisting, confirmation,
    redaction, task isolation, and approval transitions.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality`.
  - [ ] Perform and record a bare-metal/nested-KVM acceptance test, host reboot
    test, backup restore test, phone smoke test, Telegram authorization denial
    test, and approved-change smoke update test.

## Completion evidence

Record the selected host and recovery access, KVM and cgroup acceptance
results, private smoke origins, Tailscale ACL revision, systemd unit versions,
backup restore evidence, Telegram command authorization evidence, task/approval
audit samples, test counts, and any deliberately deferred follow-up tasks.
