# Purpose

`git-http` serves authenticated Git smart-HTTP traffic and the receive hook
that accepts pushes. It connects browser or runtime credentials to repository
authorization, streams Git's bounded request and response, and hands accepted
ref changes to Forge receive processing.

# Responsibilities

`GitHttpService` builds the Axum router and applies request, response, and
transaction limits before invoking the configured Git backend. Composite
authentication supports verified human identity, PAT credentials, and exact
runtime Git authority; `GitAuthorizer` then checks the repository operation
against current authorization and capability scope.

The pre-receive path validates quarantined objects, refs, ancestry, changed
paths, transfer limits, and the resolved runtime receive context before
canonical repository mutation. It carries the authenticated runtime session
and expected parent into Forge so a push cannot substitute a caller-selected
principal or bypass the receive policy.

# When

Create `GitHttpService::new` with the identity verifier, authorizer, Forge
repository, and storage, optionally install the runtime receive hook, and
serve `router()`. Use `execute_authenticated_human` for an already verified
human request and `authorize_quarantined_receive` from the receive hook.
