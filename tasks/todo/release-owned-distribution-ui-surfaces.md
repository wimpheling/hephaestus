# Release-owned distribution UI surfaces

Owner: unassigned

## Outcome

Let a published release provide a user interface that Hephaestus can mount as
a project or repository tab, or register as an explicitly installed global
interface such as the distribution's Assistant entry point. The release UI may
be either a static artifact served by Hephaestus or an application served from a
Hephaestus-managed VM. It reaches only explicitly declared gateway APIs and
inherits the platform's authorization, lifecycle, revocation, and audit
boundaries.

The first usable result may be visually minimal: a tab that hosts a same-site
iframe or opens a full-page release UI. It must nevertheless be a real,
securely bounded distribution surface, not an arbitrary external URL or an
escape hatch for custom HTML in core pages.

## Locked decisions

| Area | Decision |
| --- | --- |
| UI ownership | A release owns its routes, markup, client behavior, and domain semantics. Hephaestus owns publication, serving, authorization, lifecycle, gateway mediation, and the bounded host shell. |
| Artifact kinds | A release may declare a static UI artifact or a UI service executed in a Hephaestus-managed VM. Both are immutable release inputs and are served only after the ordinary release/runtime checks succeed. |
| Origin and routing | Hephaestus serves a release UI through a gateway-owned same-site route. Direct arbitrary third-party iframe URLs are unsupported. |
| Embedding | A bounded tab or global-interface declaration can select a scope, label, icon, route, and initial presentation (`iframe` or full page). The host rejects undeclared routes and does not let arbitrary project content alter core navigation. |
| API access | A UI reaches APIs only through explicitly declared gateway routes/capabilities. It receives no ambient database access, platform bearer token, raw secret, or authority greater than its release declaration. |
| Authentication | The host establishes the human browser session. The UI does not receive reusable human credentials; any browser-to-service identity handoff is audience-bound, short-lived, revocable, and scoped to the exact release UI route/API. |
| Isolation | Iframes use a restrictive sandbox and CSP. Static UI has no server-side execution. VM UI service authority is an exact runtime/gateway capability, not host or project authority. |
| Revocation | Removing access, disabling a release/UI tab, changing a route binding, ending a runtime, or revoking the browser session stops future serving/API use and gives a deterministic embedded failure state. |
| UI kit | Hephaestus publishes a versioned, documented UI-kit package derived from its design-system tokens/components. Projects may reuse it, but it does not expose Phoenix internals or bypass the bounded distribution API. |

## Dependencies

- Gateway routing and invocation authority from MVP-03.
- Runtime authority and release-artifact materialization from MVP-01 through
  MVP-04.
- The existing Phoenix design system remains the host-shell authority; this
  task does not loosen its raw-markup or arbitrary-class restrictions.

## Non-goals

- Arbitrary external iframe URLs, unrestricted custom HTML in Hephaestus
  pages, or loading project JavaScript into the core Phoenix origin.
- A generic chat/session protocol, generic workflow/forms engine, WebSocket
  passthrough, generic file upload, or provider-specific UI semantics.
- Giving release UIs host shell access, database access, raw secret values,
  long-lived user tokens, or cross-project/release authority.
- Replacing the existing gateway, browser authorization, or design system.

## Implementation checklist

- [ ] **1. Specify the distribution UI declaration and publication model**
  - [ ] Define a versioned release declaration for UI artifact kind, immutable
    artifact reference, scope (`project`, `repository`, or explicitly installed
    `global`), bounded route base, tab label/icon, presentation mode, declared
    gateway APIs, and compatibility behavior.
  - [ ] Validate names, paths, route ownership, icon allowlist, MIME types,
    artifact size, entrypoint, cache policy, and duplicate/conflicting tabs.
  - [ ] Persist an immutable published UI declaration tied to the release and
    expose only redacted, authorized inspection projections.

- [ ] **2. Serve static artifacts and managed UI services**
  - [ ] Materialize and integrity-check static UI artifacts from the published
    release; serve only safe declared files with deterministic MIME types,
    cache headers, range/size limits, path traversal rejection, and no
    directory listing.
  - [ ] Define the managed-VM UI service lifecycle: launch, readiness,
    health, restart/recovery, cleanup, resource limits, and failure state.
  - [ ] Route both artifact kinds through the gateway-owned same-site base;
    prevent a service from claiming another release/project route.

- [ ] **3. Add bounded host navigation and embedding**
  - [ ] Add a release/UI-tab projection to the appropriate project/repository
    page, and add an explicitly authorized global-interface projection for
    releases installed as global entries; render declared tabs through the
    existing design system.
  - [ ] Implement exact `iframe` and full-page presentations with a stable
    loading, unavailable, access-revoked, and terminated state.
  - [ ] Apply a restrictive iframe sandbox, referrer policy, CSP `frame-src`
    and `frame-ancestors` policy, and an allowlist of same-site release routes.
  - [ ] Prevent embedded content from navigating the parent, redefining core
    tabs, rendering into the host DOM, or using undeclared origins.

- [ ] **4. Bind browser and gateway authority**
  - [ ] Define the scoped browser-to-release-UI/API handoff: audience,
    expiration, replay resistance, route/release/project binding, and safe
    revocation behavior.
  - [ ] Enforce the declared API bindings at the gateway and reauthorize every
    UI route/API request against the current human/project/release state.
  - [ ] Audit allowed and denied UI serving, embeds, handoffs, and API calls
    without recording user payloads, credentials, or secret values.

- [ ] **5. Publish the reusable UI kit**
  - [ ] Choose and document a versioned package format that projects the
    design-system tokens and supported components without coupling projects to
    Phoenix internals.
  - [ ] Provide a small static and managed-service reference UI that uses the
    package, including day/night theme behavior and accessible tab content.
  - [ ] Document compatibility, upgrades, local development, release build,
    serving, and the boundary between reusable presentation and host authority.

- [ ] **6. Prove isolation, recovery, and user flow**
  - [ ] Add unit/property tests for declaration/path validation, MIME/path
    handling, artifact integrity, tab collision, handoff binding, and audit
    redaction.
  - [ ] Add real gateway/VM/static-artifact tests for authorized serving,
    service restart, failed readiness, cleanup, route conflict, and release
    removal.
  - [ ] Add browser tests for tab navigation, iframe/full-page rendering,
    dark/light theme, error/revocation state, and blocked parent-navigation or
    undeclared-origin attempts.
  - [ ] Add negative tests for cross-project/release access, stale/replayed
    handoff, undeclared APIs, arbitrary URLs, traversal, unsafe MIME handling,
    CSP/sandbox escape attempts, and raw-secret/token disclosure.

- [ ] **7. Verify and document**
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality` and `git diff --check`.

## Completion evidence

Record the declaration schema/version; static and managed-service release
fixtures; project/repository and global-interface registration evidence; artifact
integrity and route-binding evidence; browser screenshots or tests for both
presentation modes and themes; allowed/denied gateway and browser authority
fixtures; sandbox/CSP checks; revocation/restart/cleanup evidence; UI-kit
package/version and reference project; and all verification results.

Only after this task is complete may a distribution journey such as MVP-06 use
a release-owned chat tab.
