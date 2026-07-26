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

