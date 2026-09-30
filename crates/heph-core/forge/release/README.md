# Release workflow

Forge releases make build output reusable and immutable. A completed isolated
build is imported with its artifact hashes and storage identities, assigned a
release version and agent family, and then selected by a project instance.
Instances keep immutable revisions so parameters, runtime policy, capabilities,
secret bindings, and repository attachments can be compared and audited.

`release-domain` validates the values and lifecycle transitions. The release
service applies idempotent commands, resolves platform policy ceilings, and
coordinates update hooks: a candidate runs only after the current revision's
run gate drains, and uncertain hooks pause for explicit recovery.

The same release path serves UI artifacts. Installation generations have
bounded browser handoffs, active-generation routing, request projection, and
audit context so an installed UI remains tied to one release and generation.
