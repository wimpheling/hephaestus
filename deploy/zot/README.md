# Zot deployment template

This directory is the bounded deployment foundation for the forge-owned OCI
registry. It uses [Zot v2.1.18](https://github.com/project-zot/zot/releases/tag/v2.1.18),
released under Apache-2.0, as the OCI Distribution data plane. The deployment
identity is the multi-platform OCI index, not the mutable release tag:

```text
ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7
```

That index contains Linux `amd64` and `arm64/v8` manifests. The Quadlet
template selects the matching `zot-linux-{{ zot.architecture }}` binary.
The full image is deliberate: its only enabled extensions are the private
Prometheus metrics endpoint and the private Hephaestus notification sink.
Search remains disabled, which also keeps Zot management and user-preference
APIs disabled.

## Rendering inputs

Render all three `.tera` files with the same values. All values are deployment
inputs, never repository secrets.

| Input | Required value | Constraint |
| --- | --- | --- |
| `zot.architecture` | `amd64` or `arm64` | Must match the host's Linux OCI manifest. |
| `zot.private_address` | `0.0.0.0` | The Quadlet publish rule confines this listener to host loopback. |
| `zot.private_port` | private TCP port, normally `5000` | The edge and metrics scraper use this port only over the private host path. |
| `zot.storage_root` | container-private absolute path, normally `/var/lib/registry` | Its host backing directory is writable only by the Zot service UID/GID. |
| `zot.storage_host_path` | absolute host path | Dedicated filesystem/dataset; never a workspace, runtime, or general cache path. |
| `zot.config_host_path` | rendered configuration path | Mounted read-only. |
| `zot.verification_cert_host_path` | Hephaestus public verification certificate | Mounted read-only; it contains no signing key. |
| `zot.uid`, `zot.gid` | dedicated unprivileged account IDs | The storage directory must be owned or ACL-writable by these IDs. |
| `hephaestus.registry_token_realm` | public HTTPS token endpoint | Exact `WWW-Authenticate` realm served by the Hephaestus token service. |
| `hephaestus.registry_service` | configured registry authority | Exact service/audience expected by Hephaestus tokens, normally `registry.<forge-domain>`. |
| `hephaestus.registry_notification_sink_url` | private HTTP callback URL | It must resolve only from Zot's private network to the Hephaestus integration listener; never use a public forge route. |
| `hephaestus.registry_notification_callback_token` | generated unpadded base64url token | At least 43 URL-safe characters from 32 random bytes; the rendered configuration is a protected runtime secret. |
| `registry.authority` | public registry hostname | Lowercase DNS authority covered by the edge TLS certificate; no scheme or path. |
| `registry.maximum_request_bytes` | bounded upload body size | Caddy size value such as `10GB`; set from the forge's image quota. |
| `registry.upload_timeout` | bounded upload/read timeout | Caddy duration such as `30m`; large enough for the configured maximum image size. |

Tera escaping is required for all rendered JSON strings. The notification
credential is the sole secret rendered here: store the rendered configuration
in an administrator-owned `0600` path and mount it read-only. Do not render
other environment variables into this configuration: the certificate is a file
mount and the private signing key stays in the Hephaestus secret/runtime
boundary.

## Isolation contract

The generated Quadlet binds Zot only to `127.0.0.1`. The supplied Caddy route
terminates
TLS for `registry.<forge-domain>` and forwards only `/v2/` and its descendants.
It must reject `/metrics`, `/`, and Zot extension paths from public listeners.
Prometheus scrapes `http://127.0.0.1:<private-port>/metrics` through a local
or otherwise private path. `/v2/` is the readiness probe: an unauthenticated
request must return the Hephaestus Bearer challenge. Process supervision is the
liveness signal; this foundation intentionally does not create a second public
health endpoint.

Zot receives a private writable registry filesystem, read-only configuration,
and read-only token verification material. It receives no host socket, product
database credentials, token-service credentials, or token signing keys. Its
only outbound route is the private notification sink, with a short timeout and
the dedicated bearer callback credential. The edge must set the normal trusted
forwarding headers, enforce TLS, request-body and upload time limits, and avoid
exposing the loopback Zot endpoint directly.

## Required operator checks

Before installing or changing a rendered configuration, run the exact pinned
binary's `zot verify <config>`, then run
`scripts/test-zot-smoke.sh`. The smoke test renders a temporary non-secret
configuration, validates it with that binary, starts the digest-pinned image,
and exercises scoped authentication, real OCI push/pull and referrers,
authenticated notifications and callback outage, restart persistence,
missing-content behavior, private metrics, and disabled UI/search/management
paths.

Do not enable Zot garbage collection in this deployment. Disabled collection
means orphaned content consumes storage until a separately reviewed retention,
backup, restore, reconciliation, and dry-run reporting capability exists.
