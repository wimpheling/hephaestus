# Purpose

`git-credential-hephaestus` is a local Git credential helper for Hephaestus
developer personal access tokens. It translates Git's stdin/stdout protocol
into a private authority-scoped token store.

# Responsibilities

The helper accepts `get`, `store`, and `erase`, plus an explicit `login`
operation. It accepts HTTPS authorities only, hashes each authority into its
filename, validates token syntax, requires a service-owned mode `0700` root and
mode `0600` files, and zeroizes loaded token strings before they leave memory.

# When

Configure Git to call `git-credential-hephaestus` for the Hephaestus host. Git
uses `get` when it needs credentials, `store` after a successful login, and
`erase` when credentials are rejected; use `login AUTHORITY` for an explicit
local setup.
