# TODO

- [ ] Add guest Git credentials or guest-originated pushes only if a future
  interaction model requires them. Phase 4 deliberately keeps all canonical
  repository writes in the trusted host-side result publisher.
- [ ] Add interactive and human-in-the-loop agent sessions on top of durable
  run interaction requests rather than relying on long-lived VM access.
- [ ] Add controlled guest egress policies before accepting untrusted/public
  workloads.
- [ ] Add secret brokering with scoped, short-lived delivery and redaction
  before accepting untrusted/public workloads.
- [ ] Add SQLite state-volume backup, restore, and recovery verification before
  treating persistent agent state as production data.
- [ ] Resolve the provider-neutral meaning of `host_path` for remote VM
  providers. `RootFilesystem`, `VmDisk`, and `VmMount` currently carry local
  `PathBuf` values. That maps cleanly to `vm-libkrun`, which canonicalizes and
  opens files on the forge host, but a cloud provider cannot assume the same
  path exists on a remote hypervisor. Decide whether `host_path` formally means
  a supervisor-local artifact that every remote provider must securely
  stage/upload, or whether the trait needs a provider-neutral artifact
  reference with local paths as one variant. The decision must define
  allowlist validation, immutable identity or hashing, upload ownership,
  read-only guarantees, cancellation and partial-upload cleanup, caching, and
  when staged resources are released. Do not solve this by exposing
  provider-specific image IDs in the core lifecycle trait.

# DONE

- [x] Add the OIDC-authenticated Phoenix LiveView control plane for
  organizations, repositories, live run timelines, logs, metrics, artifacts,
  exact input/result commits, diffs, and durable cancel/retry/review controls.
- [x] Add controlled result proposals whose approval performs a host-side CAS
  fast-forward only when the target ref still equals the exact input commit;
  moved targets become conflicted and are never rebased or merged
  automatically.
- [x] Add a Playwright browser golden path spanning OIDC, smart-HTTP push,
  PostgreSQL RLS, NATS, run execution, live UI updates, result inspection,
  approval, and rejection.
- [x] Materialize exact accepted commits into immutable source and writable
  per-run workspaces, seal them after one-way guest finalization, safely import
  artifacts, and publish one host-controlled result ref with the exact input
  commit as parent.
- [x] Define the provider-neutral VM lifecycle, typed errors, explicit disk
  formats, networking, command, event, and cleanup contracts.
- [x] Implement the Fedora/Linux libkrun worker backend with unprivileged
  `passt`, KVM, virtio-fs, raw disks, vsock control, cgroup limits, and
  structured lifecycle tracing.
- [x] Add a deterministic fake provider and a reusable provider conformance
  test layer.
- [x] Add a non-root, digest-pinned Fedora KVM smoke-test runner with automatic
  fixture creation and leak-checked cleanup.
