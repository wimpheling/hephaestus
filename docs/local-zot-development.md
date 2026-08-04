# Local Zot development registry

`cargo dev run` starts the pinned Zot v2.1.18 image alongside the existing
local stack, then stops its container when the runner exits. Zot keeps its
state at `.local/hephaestus/zot/`, separate from repositories, VM state, and
other development resources:

- `storage/` is the persistent OCI content store;
- `config.json` is the rendered, read-only Zot configuration;
- `verification.crt` is the public bearer-token verification certificate.
- `secrets/registry-token-signing-key.pem` is the private RSA key loaded only
  by `hephaestusd`;
- `secrets/notification-callback-token` authenticates Zot's private event
  callback.

The Zot process receives the rendered configuration and public certificate,
but never the private signing key. The local state root and secret directory
are mode `0700`; private credentials are mode `0400`. The registry has no
database, NATS, host-socket, or signing-key mount. It is bound only to
`127.0.0.1`; its storage is the only writable mount.

The default endpoint is `http://127.0.0.1:55000`; set
`HEPHAESTUS_LOCAL_ZOT_PORT` to use another unprivileged port. Its bearer realm
is the Hephaestus token endpoint at
`http://127.0.0.1:8080/v1/registry/token`. Zot notifications are sent to the
authenticated private inbox endpoint at
`http://127.0.0.1:8080/internal/v1/registry/notifications`. There is no shared
registry password or authorization bypass.

The runner supplies the same loopback address to the control plane as
`HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN`. Reconciliation uses that private origin;
durable image references and token audiences still use the configured registry
authority, and browser projections never expose the private address.

`cargo dev run` does not create registry tokens on disk. A local platform
release uses the same trusted `hephaestus-registry-release` command as
production: it keeps the local signing key in the Hephaestus process boundary,
creates a five-minute (by default) token internally for one exact repository,
and writes only a temporary private OCI-client credential file. Do not create
or configure a `HEPHAESTUS_PLATFORM_TOKEN_DIRECTORY`.

For a deliberate local release, set
`HEPHAESTUS_FORGE_REGISTRY_AUTHORITY=localhost:<local-zot-port>` and set
`HEPHAESTUS_REGISTRY_SERVICE` to the identical value. Keep
`HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN` as the loopback URL with scheme; it is a
Zot read endpoint, not a registry authority or token audience. The release
command also needs the local PostgreSQL URL, local signing-key path, token
issuer/key ID, private credential root, and the pinned Skopeo/ORAS paths
documented in [the platform builder guide](../platform/builders/README.md).
It is intentionally not run by the normal development supervisor.

Use these commands to inspect it:

```sh
cargo dev status
cargo dev logs zot
cargo dev state list
cargo dev state clean --zot
```

`state init --zot` renders the existing `deploy/zot/zot-config.json.tera` and
runs `zot verify` with the exact pinned image before any service is started.
The runner repeats that validation at startup and waits for `/v2/` to return
the configured `WWW-Authenticate: Bearer` challenge. No Zot UI, search,
sync, or management endpoint is enabled by this local setup.
