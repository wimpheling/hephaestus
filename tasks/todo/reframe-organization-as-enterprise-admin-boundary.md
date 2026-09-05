# Reframe organization as the enterprise administration boundary

Owner: unassigned

## Outcome

Organizations remain the tenant, ownership, and enterprise-administration
boundary, but stop being the default day-to-day workspace. People doing
delivery work enter and operate in projects; organization surfaces make
membership, governance, portfolio discovery, and organization-owned resources
easy to find without adding navigation friction.

## Locked decisions

| Area | Decision |
| --- | --- |
| Domain model | Retain organizations as the tenant and authorization boundary. Projects remain their children; opaque identifiers and existing tenant isolation are unchanged. |
| Primary workspace | A project is the normal operational context for repositories, agents, instances, runs, and project settings. |
| Organization role | Organization views serve enterprise administration: project portfolio, members and roles, organization-owned secrets, audit/governance surfaces, and organization-level settings as they become available. |
| Navigation | Organization choice is a lightweight workspace/account switcher. Once a project is selected, normal navigation should preserve project context and not require repeated organization-page visits. |
| Authorization | This is an information-architecture change, not a relaxation of tenant boundaries or explicit delegated authority. |
| Access states | Users with organization access but no visible project retain a useful organization landing state; users with one usable project should have a direct, predictable path into it. |

## Non-goals

- Removing organizations, flattening projects into a global namespace, or
  weakening organization-scoped isolation.
- Changing the existing authorization relation model merely to simplify UI
  navigation.
- Introducing billing, SSO, or audit features that do not already have a
  product/domain implementation; provide appropriately bounded placeholders
  or defer their UI entries.

## Implementation checklist

- [ ] **1. Define the navigation and landing behavior**
  - [ ] Inventory every organization entry point, breadcrumb, redirect, and
    empty state in the browser control plane.
  - [ ] Specify the deterministic destination after organization selection for
    zero, one, and multiple visible projects, including deep-link and denied
    access behavior.
  - [ ] Make project context persistent and readily switchable without
    obscuring the owning organization.

- [ ] **2. Make project work primary**
  - [ ] Update global navigation, breadcrumbs, and project links so common
    repository, agent, instance, run, and settings journeys start or continue
    in a project.
  - [ ] Replace organization-page copies and calls to action that imply it is
    the normal work surface with project-oriented language.
  - [ ] Preserve clear, accessible routes for switching organization and
    project from any project-scoped page.

- [ ] **3. Focus the organization surface on administration**
  - [ ] Present visible projects as a concise portfolio/discovery view rather
    than a second operational dashboard.
  - [ ] Group existing organization-owned secrets and membership/governance
    actions under an explicit administration context.
  - [ ] Define bounded navigation slots and empty states for future enterprise
    administration capabilities without exposing unavailable controls.

- [ ] **4. Preserve authorization and accessibility**
  - [ ] Keep organization, project, repository, secret, and capability
    authorization checks at their current resource boundaries.
  - [ ] Add focused UI and route-model tests for one-project direct entry,
    multi-project selection, no-project organization landing, project switch,
    deep links, denied access, and revocation/reconnect behavior.
  - [ ] Run accessibility checks for changed switcher, navigation, breadcrumb,
    and landing-state controls.

- [ ] **5. Document and verify**
  - [ ] Update product/navigation documentation to distinguish the
    organization administration boundary from the project work surface.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality` and `git diff --check`.

## Completion evidence

- [ ] Record the verified commands, focused browser/accessibility coverage,
  documentation updates, and any intentionally deferred enterprise
  administration surfaces before moving this task to `tasks/done/`.
