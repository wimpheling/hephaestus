# TODO

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

- [x] Define the provider-neutral VM lifecycle, typed errors, explicit disk
  formats, networking, command, event, and cleanup contracts.
- [x] Implement the Fedora/Linux libkrun worker backend with unprivileged
  `passt`, KVM, virtio-fs, raw disks, vsock control, cgroup limits, and
  structured lifecycle tracing.
- [x] Add a deterministic fake provider and a reusable provider conformance
  test layer.
- [x] Add a non-root, digest-pinned Fedora KVM smoke-test runner with automatic
  fixture creation and leak-checked cleanup.
