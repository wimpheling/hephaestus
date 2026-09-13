# Contributor Instructions

## Product target

The current product target is documented in the [product roadmap](tasks/roadmap.md):
“Ship a Heph distribution whose primary entry point is an agent UI that helps
users administer their Heph instance and create, code, and run projects for
them.” Keep this target distinct from implementation status; task locations and
historical records remain authoritative.

## Agent methodology

- Use `gpt-6-astra` with `medium` reasoning effort only for the main thread's
  orchestration, architectural decisions, concise review, and final integration
  review: user discussion, acceptance criteria, task decomposition, and resolving
  ambiguity.
- Use `gpt-5.6-luna` with `high` reasoning effort for subagents owning bounded
  coding tasks, repository exploration, information gathering, focused analysis,
  test implementation, running checks, and failure investigation.
- Give each agent one bounded task at a time with precise scope, relevant
  context, file ownership, and an expected deliverable. Avoid duplicate
  investigations and do not take execution back into Astra while a bounded task
  is in progress.
- Agents should return concise evidence summaries with verification results and
  unresolved questions. Prefer completion notifications and longer waits over
  frequent polling. Reassign or refine a bounded task when it stalls, and
  escalate design decisions or ambiguity to the main thread.
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

The durable configuration and debugging procedure for the disposable GCP
Cooking CI modes is [`docs/gcp-cooking-ci.md`](docs/gcp-cooking-ci.md).
