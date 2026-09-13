# Ship an agent-led Heph distribution

Owner: unassigned

## Outcome

Ship a Heph distribution whose primary entry point is an agent UI that helps
users administer their Heph instance and create, code, and run projects for
them.

The distribution supplies a bundled, replaceable assistant with a documented
starter project and sensible defaults. The assistant can inspect installation
state, propose or perform authorized management actions, create or import an
ordinary project, help make source changes, run checks and project workloads in
the existing isolated runtime, and present durable results. The trusted shell
retains authentication, authorization, capability grants, deployment and
recovery approvals, and a deterministic path for operating Heph when the
assistant is unavailable.

Projects are not limited to agent projects. Maintained templates are the first
creation path; arbitrary reliable agent generation is outside this task. The
assistant and UI are ordinary release-owned software and do not become a new
core conversation, workflow, or privileged-proxy architecture.

## Current evidence and gaps

The repository already has these foundations:

| Area | Current evidence | Status for this outcome |
| --- | --- | --- |
| Isolated execution | Exact-commit releases and instances, capability ceilings, runtime authorization, and VM execution are implemented in [MVP-01](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md) and [releases and instances](../../docs/releases-and-instances.md). | Implemented foundation; integrated assistant journey is not proven. |
| Coding and results | Writable per-run workspaces, safe sealing/import, Git result publication, artifacts, and provenance are implemented in [workspaces and results](../../docs/workspaces-and-results.md). | Implemented foundation; assistant-driven project coding/run flow is not proven. |
| Manual management | Phoenix provides organization/project/repository/agent inspection and controls through the typed control plane. | Implemented foundation; this does not establish a packaged assistant entry point. |
| Interactive session | MVP-06 specifies a release-owned Git-backed chat reference journey. | Planned precursor; its joint plan review and implementation remain open. |
| Default distribution | No bundled bootstrap/default assistant has been proven to install and open as the primary entry point. | Missing. |
| Delegated administration | The platform documents human-mediated, short-lived delegation and one-shot approval direction, but the packaged assistant path is not implemented. | Missing/unverified. |

Existing runs are attachment and push driven through the current typed contracts
and Git triggers. Reuse those operations during the interactive-path audit;
introduce a new run RPC only if the audit demonstrates that the existing
contracts cannot support the journey. A persistent gateway/service runtime is
conditional on the chosen project interface and is not a prerequisite for all
batch project runs.

## Dependencies

- [Product roadmap](../roadmap.md) and [own-the-loop product definition](define-own-the-loop-agent-platform.md).
- Completed release, capability, runtime Git, workspace/result, and control-plane
  foundations, especially [MVP-01](../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md),
  [MVP-01.2](../done/mvp-01.2-replace-controlled-result-publication-with-runtime-git.md),
  and [MVP-04](../done/mvp-04-brokered-model-and-outbound-capabilities.md).
- [Release-owned distribution UI surfaces](release-owned-distribution-ui-surfaces.md),
  including explicitly authorized global-interface registration.
- The [MVP-06 Git-backed session chat journey](../in-progress/mvp-06-git-backed-session-chat-journey.md)
  as an interaction precursor, after its required joint plan review; MVP-06 is
  not itself completion of this outcome.
- [Browser reconnect and restart verification](verify-browser-reconnect-and-restart.md)
  for the durable visible journey.

## Locked boundaries

| Area | Decision |
| --- | --- |
| Primary entry | The default distribution opens the agent UI first. Its exact packaged agent split (Operator/Admin Agent, Project Agent, or another bounded arrangement) must be resolved before implementation. |
| Human authority | The interacting human remains the effective principal. The assistant is a mediator under a short-lived delegation ceiling; it cannot grant itself authority or approve its own sensitive proposal. |
| Trusted controls | Authentication, authorization, grants, deployment, recovery, and break-glass controls remain available through the trusted shell with server-side checks. |
| Project creation | The first path uses maintained, inspectable templates and ordinary repositories. The project may be any supported software project, including but not limited to an agent. |
| Source and execution | Use existing Git, release, attachment, run, workspace, artifact, and result contracts. Do not add a browser IDE or generic file service unless a focused audit proves one necessary. |
| Conversation ownership | The visible transcript, agent-owned model context, and internal workflow state must be identified separately. A release may own its session protocol; Hephaestus does not assume a universal one. |
| Security | The assistant receives only typed, scoped APIs and capabilities. It has no ambient database, host shell, master credential, or generic privileged proxy. |

## Observable acceptance journey

- [ ] Install or initialize the default distribution from a documented,
  versioned bootstrap, and open the configured global Assistant entry after
  authenticated login.
- [ ] Ask the assistant to inspect the Heph installation and show bounded,
  authorization-filtered health, project, agent, run, and diagnostic state.
- [ ] Have the assistant propose one bounded, supported instance-management
  change (such as changing an attachment or requesting instance recovery), show
  the exact scope in the trusted shell, obtain any required human approval,
  execute it under live authorization, and verify the resulting state and audit
  provenance in the assistant UI.
- [ ] Create a project from a maintained template or import an existing project,
  show the source and required capabilities, and preserve project context.
- [ ] Make a visible source customization through the supported Git/project
  workflow. Record the exact source revision and keep generated changes
  reviewable.
- [ ] Request a project check or run. Execute it through the existing isolated
  runtime and show working, completed, failed, and denied states with links to
  exact runs, logs, artifacts, and results.
- [ ] Require trusted-shell, server-authorized approval for capability grants,
  release/deployment changes, and other sensitive actions; show the proposed
  scope and resulting provenance without exposing secrets.
- [ ] Reconnect the browser and assistant UI after a process or channel restart,
  resume the durable visible history exactly once, and retain the correlation
  between user request, accepted Git/run operation, execution, and result.
- [ ] Operate installation, recovery, and privileged approvals through
  deterministic shell controls when the assistant is stopped, unavailable, or
  revoked.

## Prioritized implementation checklist

- [ ] **P0: Resolve and implement the primary entry point**
  - [ ] Decide the packaged Operator/Admin Agent and Project Agent boundary,
    lifecycle, replacement/removal behavior, and global Assistant registration.
  - [ ] Define the versioned assistant UI/release declaration and the narrow
    browser-to-release bridge using the release-owned UI task.
  - [ ] Define bootstrap defaults, installation/recovery behavior, and a
    break-glass path that does not depend on the assistant.

- [ ] **P0: Provide bounded delegated management**
  - [ ] Expose only typed inspection, project creation/import, source/run
    coordination, and proposal operations needed by the journey.
  - [ ] Enforce human-effective-principal attribution, delegation ceilings,
    one-shot approvals, live authorization, RLS, and redacted audit for every
    privileged request.
  - [ ] Prove denial for cross-project access, undeclared capabilities,
    self-granting, self-approval, raw credentials, and host/database access.

- [ ] **P0: Integrate the project create/code/run loop**
  - [ ] Implement the template/import flow over existing project, repository,
    release, attachment, and capability operations.
  - [ ] Integrate source customization and test/run requests using existing Git
    triggers and runtime/workspace/result contracts wherever they suffice.
  - [ ] Link durable requests to exact runs, logs, artifacts, results, and
    release/instance provenance in the assistant UI.

- [ ] **P1: Prove interactive durability and usability**
  - [ ] Complete the MVP-06 precursor at its reviewed scope, including failure,
    restart, and release-owned session-history behavior.
  - [ ] Complete browser reconnect/restart evidence for the final assistant UI,
    including duplicate suppression and authorization-safe denial states.
  - [ ] Provide starter templates, coding guidance, project diagnostics, and
    replacement/version compatibility documentation.

- [ ] **P2: Validate the distribution**
  - [ ] Put the distribution in front of developers building ordinary projects
    and record task-specific usability blockers.
  - [ ] Measure whether the assistant meaningfully reduces manual resource
    wiring while preserving visible authority and approval boundaries.
  - [ ] Define later retention, backup, catalog, and service-runtime work only
    where the validated journey requires it.

## Non-goals

This task does not define a universal model prompt, session protocol, workflow
engine, browser IDE, arbitrary external iframe, unrestricted project JavaScript,
new scheduler, second run/delivery system, or automatic reliable generation of
arbitrary agents. It does not make a functioning assistant necessary for
installation, recovery, grants, deployment approvals, or break-glass operation.

It does not require persistent gateway service mode for batch project runs;
that remains the separately scoped [persistent gateway runtime task](persistent-gateway-service-runtime-and-development-workflow.md).

## Verification and completion evidence

- [ ] Record the bootstrap/distribution manifest and exact packaged release
  source revisions.
- [ ] Record browser evidence for primary global entry, inspection, project
  creation/import, source customization, run/result display, approvals, failure
  states, and reconnect/restart.
- [ ] Record exact authorization/delegation decisions, denied-capability tests,
  redaction scans, and break-glass operation while the assistant is unavailable.
- [ ] Run focused Rust, Phoenix, browser, and integration checks for changed
  boundaries, then run `git diff --check` and `cargo dev quality` at handoff.
