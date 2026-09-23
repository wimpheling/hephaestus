# Add parent-scoped resource slugs

Owner: unassigned

## Outcome

Organizations, projects, and repositories have stable human-readable URL slugs
using lowercase ASCII letters, digits, and hyphens. Each slug is unique only
within its owning parent, following the GitHub-style namespace model.

## Locked decisions

- A slug matches `^[a-z0-9]+(?:-[a-z0-9]+)*$`.
- Organization slugs are globally unique.
- Project slugs are unique within an organization.
- Repository slugs are unique within a project.
- Opaque UUIDs remain the canonical internal identifiers and authorization
  subjects; slugs are routing and display identifiers only.
- Git remote paths and browser routes must not embed credentials.

## Implementation checklist

- [ ] Add immutable or explicitly-renamable slug columns, parent-scoped unique
  constraints, safe migration/backfill rules, and conflict diagnostics.
- [ ] Update create/edit commands and UI forms to validate and display slugs.
- [ ] Add canonical browser routes for organization, project, and repository
  slugs, with UUID route compatibility and redirects during migration.
- [ ] Define canonical Git smart-HTTP paths using the parent slug hierarchy,
  while continuing to resolve authorization through opaque IDs.
- [ ] Decide and implement rename behavior, aliases/redirect lifetime, and
  collision handling without breaking historical release/run provenance.
- [ ] Add coverage for validation, uniqueness at each parent boundary,
  authorization after slug lookup, rename/redirect behavior, and Git clone/push
  over canonical slug paths.

## Non-goals

- Replacing UUIDs in database relations, events, audit records, or runtime
  authority.
- Allowing arbitrary Unicode, whitespace, underscores, or path separators in
  canonical slugs.
