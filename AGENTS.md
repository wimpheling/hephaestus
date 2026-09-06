# Contributor Instructions

## Agent methodology

- Use `gpt-6-astra` with `medium` reasoning effort for the main thread and
  orchestration: user discussion, architectural analysis, acceptance criteria,
  task decomposition, resolving ambiguity, and final review.
- Use `gpt-5.6-luna` with `high` reasoning effort for subagents handling bounded
  coding tasks, repository exploration, information gathering, focused analysis,
  test implementation, running checks, and failure investigation. Delegate
  substantial execution work when it can proceed independently alongside useful
  main-thread work.
- Give each subagent a precise scope, relevant context, and an expected result.
  Coordinate file ownership for concurrent edits and avoid duplicating work.
  Handle tiny tasks directly when delegation overhead exceeds the work.
- Subagents should return evidence, verification results, and unresolved
  questions. Escalate design decisions, unclear requirements, and repeated stalls
  to the main thread. Astra remains responsible for reviewing the integrated
  result against the acceptance criteria.
- Select the model and reasoning effort explicitly when spawning subagents.
  This file records the policy; the main-thread model and effort must be selected
  in the session or runtime configuration, not changed by these instructions.

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
