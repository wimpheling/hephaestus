# Capability permissions

Released agents declare symbolic capability slots. A declaration describes a
purpose, one controlled resource kind, required operations, optional
operations, and whether the slot itself is required. Release source cannot
name a tenant resource or carry a grant.

An imported project instance remains unrunnable while a required slot is
unbound. An authorized user completes the capability review on the instance:

1. select one exact resource from the choices they may both use and delegate;
2. review the release-required operations;
3. explicitly select any optional operations; and
4. confirm the complete set.

Confirmation creates and activates a new immutable instance revision. The
server rejects stale revisions, undeclared slots, incompatible resource kinds,
missing required operations, undeclared optional operations, cross-project
resources, and any operation the user cannot grant. Changing or removing a
binding repeats this process and preserves the historical revision.

The active binding is only the maximum authority. Each privileged runtime call
must also pass current authorization and resource-lifecycle checks. The
instance page shows the release requirement, exact resource, granted operation
set, grantor, current live status, last use, redacted runtime sessions, and
recent redacted authorization evidence. Runtime credentials, credential
verifiers, request bodies, secret values, and provider authorization material
are not exposed by this inspection surface.

Instance LiveViews use page-scoped product-event wakeups. Every wakeup causes a
fresh authorized snapshot read; a subscription event is never itself treated
as authorization to reveal updated capability state.

Repository capabilities may additionally require a typed Git ref/path scope.
The generic capability form cannot invent or broaden that scope; runtime Git
configuration is accepted only through the typed Git authority contract.
