# Purpose

`pat-domain` models a developer personal access token from issuance through
expiry or revocation. It gives a user a labeled, bounded Git scope that can be
listed as safe metadata and checked at each use.

# Responsibilities

`PersonalAccessToken` is the bearer shown once to the user; its verifier and
`PersonalAccessTokenRecord` are the durable comparison and lifecycle state.
`PersonalAccessTokenScope` limits Git operations and can restrict them to
selected repositories. `authorize_at` checks token identity, verifier, owner,
active lifetime, operation, and repository before allowing a use, while
`record_use` and `revoke` update the lifecycle monotonically.

The domain bounds token lifetime and label size, redacts bearer material from
formatting and serialization, and returns safe denial errors without including
the token. The caller still combines the token-local result with the owner's
current repository authorization.

# When

Use this crate when a user creates or presents a developer Git token. Issue a
`PersonalAccessToken`, create its `PersonalAccessTokenRecord`, and call
`authorize_at` for each requested repository operation before recording a
successful use.
