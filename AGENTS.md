# Contributor Instructions

## Rust quality policy

- Keep the workspace's strict Rust, Clippy, and rustdoc lint configuration
  enabled.
- Treat Clippy's `pedantic` lint group as the default baseline.
- Soften checks when they reduce clarity, conflict with a deliberate design, or
  impose an unreasonable maintenance cost.
- Prefer the narrowest possible exception: allow a specific lint on the
  smallest relevant item instead of weakening it workspace-wide.
- Add a brief comment explaining every lint exception that is not
  self-explanatory.
- Run formatting, Clippy, tests, and documentation checks before considering a
  Rust change complete:

  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features
  cargo test --workspace --all-features
  cargo doc --workspace --all-features --no-deps
  ```

For the repository-wide handoff gate, run `cargo dev quality`; it composes
generated-code/Buf validation, architecture, Rust, Phoenix, UI, and focused
integration checks in one command.

Common architecture violations have direct remediations: keep SQLx and SQL in
declared PostgreSQL adapters, convert generated RPC types at the transport
boundary, publish product events only through committed outboxes, and mark
request-only plaintext with the protobuf sensitive-field option. The linked
architecture rule index is the source of truth for diagnostics and exceptions.
