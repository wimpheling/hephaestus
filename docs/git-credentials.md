# Developer Git credentials

Developer Git over HTTPS uses personal access tokens created at
`/settings/git-credentials`. Browser authentication remains OIDC-based. A PAT
is shown once after creation or rotation; Hephaestus persists only a one-way
verifier and safe lifecycle metadata.

Install `git-credential-hephaestus` on the developer machine and register the
helper globally:

```sh
cargo install --path crates/git-credential-hephaestus
git config --global credential.helper hephaestus
```

This stores only the helper name in global Git configuration. It never places
the bearer token in a remote URL or repository configuration. Copy the
one-time value from the browser and provide it on standard input:

```sh
git-credential-hephaestus login git.example.com
```

The helper accepts only HTTPS authorities, stores each authority in a separate
mode-`0600` file under the user's data directory, and returns a token only for
an exact authority match. `HEPHAESTUS_GIT_CREDENTIAL_ROOT` may select an
absolute alternative storage directory. After rotating a PAT, run `login`
again with the replacement. Revoking a server-side PAT takes effect
immediately; remove the local copy with Git's standard credential rejection
flow or `git-credential-hephaestus erase` using the Git credential protocol.

PAT scopes narrow allowed Git operations and optionally exact repository IDs.
They never replace the owner's live repository authorization.
