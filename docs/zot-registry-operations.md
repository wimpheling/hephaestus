# Forge-owned Zot registry operations

## Scope and trust boundary

Zot is the isolated OCI Distribution data plane for Hephaestus. It owns OCI
blobs, manifests, indexes, tags, referrers, upload sessions, and local storage
state. Hephaestus owns every product decision: namespace ownership,
authorization, short-lived token issuance, approval, catalog registration,
reconciliation, and product events. A Zot response only proves bytes exist; it
does not approve or authorize their use.

The deployment foundation is in `deploy/zot/`. It pins Zot v2.1.18 by OCI
index digest:

```text
ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7
```

The source release is [project-zot/zot v2.1.18](https://github.com/project-zot/zot/releases/tag/v2.1.18),
licensed under Apache-2.0. Its index includes Linux `amd64` and `arm64/v8`
deployment images. An upgrade changes the release tag and index digest together,
is validated with the pinned binary's `zot verify`, and passes the smoke test
before production rollout. Never use a tag in a deployment file or rely on
automatic image updates.

## Configuration inputs

Render `deploy/zot/zot-config.json.tera`, `zot.container.tera`, and
`registry-edge.caddy.tera` from one administrator-controlled Tera model. The
required inputs and their constraints are listed in the deployment README. In
particular:

- `registry_token_realm` is the exact public HTTPS Hephaestus token-service
  endpoint placed in the OCI Bearer challenge.
- `registry_service` is the exact configured registry authority/audience. The
  token service and Zot must agree on it; do not use a shared generic service
  name.
- `verification_cert_host_path` holds a public PEM certificate or public key
  emitted by the Hephaestus registry-token service. Zot uses it only to verify
  bearer tokens. The signing key stays outside Zot.
- `storage_host_path` is a dedicated local filesystem or dataset, mounted only
  at `/var/lib/registry`. The initial deployment uses no remote backend,
  pull-through cache, mirror, or storage subpath.
- `registry_notification_sink_url` is an internal-only Hephaestus endpoint.
  Zot can reach it over its private service network, but it is neither an edge
  route nor a browser-facing API.
- `registry_notification_callback_token` is 32 random bytes encoded as an
  unpadded URL-safe base64 string. It is rendered into Zot's `0600`, read-only
  configuration as the HTTP sink bearer token and is separately supplied to
  the Hephaestus integration runtime. Rotate it as a callback credential, not
  as a registry-client token.

The template sets `gc: false`, `commit: true`, and deduplication. It enables
the private Prometheus extension at `/metrics` and Zot's HTTP events extension.
Metrics are private operational data, not a public product endpoint. Search,
UI, and sync are explicitly disabled; Zot derives management and
user-preference API availability from search, so those APIs are disabled with
it. Omitted extensions, including trust, lint, scrub, and embedded API-key
authentication, remain disabled.

## Notification ingestion and reconciliation

Zot v2.1.18 emits HTTP events as binary-mode CloudEvents. The sink sends a
JSON data body with `Ce-Specversion: 1.0`, `Ce-Source: zotregistry.dev`, a
UUID `Ce-Id`, `Ce-Type`, and `Ce-Time`; it uses
`Authorization: Bearer <registry_notification_callback_token>`. The supported
Zot event types are repository creation, image update, image deletion, and
image lint failure. The payload can include actor and request metadata, but
Hephaestus validates and discards those fields.

The integration endpoint must be private, permit `POST` only, limit headers and
the body, authenticate the dedicated callback bearer credential before parsing,
and pass the request to `registry-notification`. That boundary accepts only the
documented Zot source, event types, canonical repository paths, SHA-256
digests, OCI media types, and bounded timestamps. It derives its durable
idempotency key from `Ce-Source` plus `Ce-Id` and hashes the exact body with
SHA-256. It retains only the event key, body hash, compact routing metadata,
and timestamp in `registry_notification_inbox`; it never retains the raw body,
manifest, actor, request address, user agent, or credential.

Zot publishes notifications asynchronously and only logs sink failures; its
HTTP sink does not make callbacks durable or retry them. Delivery is therefore
best-effort, and duplicate, delayed, reordered, and missed callbacks are all
expected. An accepted callback only schedules/reinforces reconciliation. The
reconciler reads the manifest and referrers back from Zot by digest and only
the successful Hephaestus lifecycle transaction may approve content and append
a product outbox event.

## Network and process isolation

The Zot Quadlet binds its port to `127.0.0.1` only. The forge edge terminates
TLS for `registry.<forge-domain>` and forwards only `/v2/` and descendants. It
must preserve `Host` and only trusted forwarding headers, enforce request body,
header, and upload time limits, and reject `/metrics`, `/`, UI, search, and
management routes on every public listener.

Prometheus uses the private loopback endpoint. `/v2/` is both the OCI ping and
the readiness check: without a token it must return `401` with a `Bearer`
challenge containing the configured realm and service. Treat process
supervision as liveness. No independent public health route is configured.

Run Zot as its dedicated non-root account with a read-only root filesystem,
all Linux capabilities dropped, a writable registry-storage mount, and
read-only configuration and verification-certificate mounts. It must not have
a container/host socket, repository workspace, product database or NATS
credentials, token-service credentials, or the token signing key.

## Authorization and namespaces

Hephaestus issues short-lived OCI bearer tokens with a single exact repository
and only the actions already authorized for the caller. Zot validates their
signature using the mounted verification material. Repository ownership is
never expressed by mutable human names:

```text
platform/images/<image-key>
projects/<project-uuid>/repository-images/<image-uuid>
projects/<project-uuid>/release-agents/<release-agent-uuid>
```

The token service must deny cross-project reads, mounts, tag/referrer
enumeration, and pushes unless a future explicit grant authorizes them.
Consumers persist and use only `authority/path@sha256:<64 lowercase hex>`;
tags are optional discovery aliases and never execution inputs.

## Trusted platform publication

Platform OCI images are released through `scripts/publish-platform-images.sh`,
which invokes the administrator-only `hephaestus-registry-release`
subcommand for each reviewed OCI layout. The command is part of the forge
control plane, not a general registry client: it creates or resumes the
publication intent, issues its own short-lived RS256 `pull,push` token for the
one exact `platform/images/<key>` namespace, performs the controlled
publication, reads Zot back by digest, records verification, and commits
approval. There are no pre-issued `.jwt` files or shared registry passwords.

The release operator must provide the following protected runtime
configuration. `HEPHAESTUS_REGISTRY_SERVICE` must exactly equal
`HEPHAESTUS_FORGE_REGISTRY_AUTHORITY`; the authority contains no scheme or
path. The signing key and database connection stay in the Hephaestus runtime
boundary and are never mounted into Zot.

```sh
export HEPHAESTUS_FORGE_REGISTRY_AUTHORITY=registry.forge.example
export HEPHAESTUS_REGISTRY_SERVICE=registry.forge.example
export HEPHAESTUS_DATABASE_URL='postgres://…'
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY=/run/hephaestus/registry-signing-key.pem
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER=https://forge.example/v1/registry/token
export HEPHAESTUS_REGISTRY_TOKEN_KEY_ID=registry-2026-08
export HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS=300
export HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT=/run/hephaestus/platform-builder-credentials
export HEPHAESTUS_REGISTRY_RELEASE=/opt/heph-tools/hephaestus-registry-release
export HEPHAESTUS_REGISTRY_RELEASE_VERSION='hephaestus-registry-release 0.x.y'
```

The wrapper sets `HEPHAESTUS_REGISTRY_LAYOUT_ROOT` to its private input root.
The release command also uses the pinned absolute `HEPHAESTUS_SKOPEO` and
`HEPHAESTUS_ORAS` configured by the platform-builder documentation. The
credential root is for private temporary OCI-client auth files only; it must
not be used as a token distribution directory.

SPDX SBOM, in-toto provenance, and vulnerability-scan referrers are required.
A verified signature/approval referrer is optional under the current
`without_signature` policy. If supplied, it is recorded and verified; if not,
the release is still eligible for approval and its catalog signature field is
`null`. Changing that policy to require a signature is a separate reviewed
control-plane change.

## Operations

For each config or image change, render into an administrator-owned path, make
the certificate mount readable but not writable by Zot, and run:

```sh
scripts/test-zot-smoke.sh
```

The smoke test uses an ephemeral local signing key/certificate and a non-secret
fixture callback token solely to validate the configuration. It verifies: the
exact image starts read-only, valid and invalid configurations and verification
keys behave correctly, `/v2/` emits the expected Bearer challenge, pull-only
credentials cannot push, and authenticated Skopeo push/pull works. It also
uploads and reads an OCI 1.1 referrer, verifies authenticated CloudEvents
delivery, proves a callback outage does not block publication, restarts Zot and
reads the persisted image and referrer, checks missing-content behavior, reads
private metrics, and rejects UI/search/management surfaces. It does not contact
a production registry or replace the upstream OCI conformance suite.

Alert on unavailable `/v2/` readiness, auth failures, request errors/latency,
upload duration, and filesystem capacity. Do not log bearer tokens, credentials,
private manifest bodies, or verification-key material. Capacity planning must
include orphaned objects because automatic garbage collection remains disabled.

Back up the dedicated Zot storage consistently with the Hephaestus PostgreSQL
metadata. On restore, keep publication and execution disabled, restore storage
and metadata, then reconcile every approved digest and required referrer
against Zot before allowing pulls or materialization. A missing approved blob
or manifest fails execution closed. Retention and any destructive collection
require a separate reviewed change with retention roots, dry-run reports,
restore tests, and reconciliation evidence.

## Non-destructive retention report

`hephaestus-operator registry-retention-report <inventory.json>` compares a
bounded descriptor inventory exported from the private Zot operational boundary
with the durable Hephaestus publication roots. The command is read-only: it has
no Zot credentials, storage path, or deletion API and always emits
`"mode":"report_only"`.

The versioned input contains canonical repository paths and descriptor digests:

```json
{
  "schema_version": 1,
  "entries": [
    {
      "repository_path": "platform/images/rust-ubuntu",
      "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    }
  ]
}
```

The report protects active intents, approved platform catalog images,
project OCI images, release agents, their platform manifests, and required
referrers. It separately lists durable roots missing from the supplied
inventory and observed descriptors that have no durable root. Its
`schema_scope` flags are deliberately explicit: generic build-image and generic
release-artifact OCI roots remain false until those product records actually
own OCI publications. Never interpret `unreferenced_inventory` as deletion
authorization.

The same report includes lifecycle and durable notification-inbox counters.
Alert whenever missing publications, expired notification claims, or rejected
notifications are non-zero; alert on sustained pending/publishing backlog and
verified publications awaiting review. Combine those control-plane counters
with Zot's private request/error/upload metrics and filesystem monitoring.
Deployment policy must set a hard per-request upload limit at the edge, a
filesystem capacity warning and critical threshold, per-project namespace
growth budgets, a maximum upload duration, and a maximum concurrent-upload
budget. Because Zot garbage collection is disabled, hitting a storage budget
blocks new publication rather than deleting content automatically.

Rotate verification material by publishing a new public verifier through the
existing Hephaestus secret/runtime boundary, using overlapping token-validation
windows, atomically replacing the read-only mounted file, restarting Zot, and
then retiring the old signer only after all issued tokens can no longer be
accepted. The Zot service must never receive either private key.

Rotate the notification callback credential independently. First deploy the
new secret to the Hephaestus integration runtime while it accepts the old and
new callback credentials for a short overlap. Atomically replace Zot's
read-only rendered configuration and restart Zot, then retire the old callback
credential after the overlap. Never write either credential to logs, metrics,
or the durable inbox.
