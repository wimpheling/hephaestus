# Release UI package and serving guide

This guide describes the release-owned UI package and the two Cooking reference
fixtures. It covers the repository-local build and the daemon's opt-in UI
origin. It does not claim that an installed Caddy, VM, TLS, or browser
acceptance run has passed; that proof is still a separate release check.

## Package compatibility

The current package is `@hephaestus/release-ui-kit` version `1.0.0`. Its
generated files are immutable, versioned assets:

```text
heph-ui-kit-v1.0.0.css
heph-ui-kit-v1.0.0.js
manifest.json
```

The CSS uses the `heph-ui-` class prefix. The optional helper is presentation
only: it validates one `heph_theme=light|dark` value and an exact HTTPS
`heph_theme_origin`, applies the theme, and exchanges the closed ready/theme
messages described in the kit README. It does not provide authentication,
cookies, storage, navigation authority, or platform data APIs.

Release declarations currently use `ui_kit_version = 1` for this package. The
static fixture declares `index.html`, CSS, and helper artifacts separately
with their MIME types. The managed fixture declares the same kit version on an
authenticated `http.service.v1` route and names its `index.html` entrypoint.

## Upgrades

Publish a new kit version under new filenames and a new manifest, then update
the release declaration and vendored fixture copy together. Keep the old
versioned files available for releases that still reference them; do not
replace the bytes behind an existing version. Re-run the package checks and
the fixture build checks before publishing a release. The current repository
does not define a compatibility matrix beyond version 1, so a future kit
version must document any changed helper message or CSS contract alongside its
new version.

## Local development and release builds

The kit checks are offline and do not download dependencies:

```sh
cd web/assets/release_ui_kit
npm run test
npm run check
```

Build each reference fixture into its own disposable absolute directory. The
scripts verify the vendored kit manifest and its CSS/helper hashes before
copying release artifacts:

```sh
static_output=$(mktemp -d)
managed_output=$(mktemp -d)
trap 'rm -rf "$static_output" "$managed_output"' EXIT
HEPHAESTUS_REFERENCE_UI_OUTPUT="$static_output" \
  examples/cooking/cooking-reference-ui/build.sh
HEPHAESTUS_REFERENCE_SERVICE_UI_OUTPUT="$managed_output" \
  examples/cooking/cooking-reference-service-ui/build.sh
```

The static build writes `dist/index.html` and the versioned kit assets plus
`bin/reference-ui-check`. The managed build writes the service, its HTML, and
the same versioned assets under `bin/`. The build scripts require absolute
output paths and use the checked-in vendored inputs; they are suitable for a
release build, not a replacement for installed acceptance.

For a local managed-service smoke, start the copied service from its `bin`
directory and inspect its bounded fixture endpoints:

```sh
(cd "$managed_output/bin" && exec python3 ./reference-ui-service) &
service_pid=$!
trap 'kill "$service_pid" 2>/dev/null || true; rm -rf "$static_output" "$managed_output"' EXIT
curl --fail http://127.0.0.1:8080/readyz
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/reference/index.html
curl --fail http://127.0.0.1:8080/reference/heph-ui-kit-v1.0.0.css
curl --fail http://127.0.0.1:8080/reference/heph-ui-kit-v1.0.0.js
```

The release fixture declares a network-disabled guest profile and serves no
credentials or state. The local Python process itself only binds to loopback;
it does not disable the host network. Stop it and remove the temporary output
directory after the smoke. This local process does not exercise the release authority,
PostgreSQL verifier, Caddy, TLS, or an authenticated browser session.

## Release declarations and serving

The static reference declaration is in
`examples/cooking/cooking-reference-ui/heph.ui.toml`:

```toml
presentation = "full_page"
route_base = "reference"
ui_kit_version = 1
```

The managed declaration is in
`examples/cooking/cooking-reference-service-ui/heph.ui.toml`:

```toml
presentation = "iframe"
route_base = "managed-reference"
ui_kit_version = 1

[uis.content]
kind = "managed_service"
route = "/reference"
entrypoint = "index.html"
```

The route base and entrypoint are part of the release declaration. Relative
asset links in both reference documents therefore resolve through the
authenticated canonical entrypoint path. Static artifacts remain immutable;
managed content is served by the declared gateway service. The `no_store`
cache policy in both current declarations is intentional.

## Host authority and daemon configuration

The daemon UI origin is opt-in. Leaving all UI-specific variables unset keeps
the ordinary daemon path unchanged. Enabling it requires the existing Caddy
configuration and these variables:

```text
HEPHAESTUS_CADDY_ADMIN_URL
HEPHAESTUS_UI_ORIGIN_LISTEN
HEPHAESTUS_UI_NAMESPACE
HEPHAESTUS_PLATFORM_HTTPS_ORIGIN
```

`HEPHAESTUS_UI_PORT` is optional and defaults to canonical HTTPS port 443. The
listener must be a nonzero loopback address. `HEPHAESTUS_UI_NAMESPACE` must be
a strict DNS subdomain of the host in the exact HTTPS
`HEPHAESTUS_PLATFORM_HTTPS_ORIGIN`; DNS case is canonicalized and noncanonical
ports, paths, queries, fragments, aliases, and trailing dots are rejected.
The public UI port is independent of the private loopback listener. If any UI
setting is supplied, partial UI configuration fails startup.

The existing Caddy owner remains responsible for public TLS and forwarding to
the dedicated loopback listener; the daemon does not create a second Caddy
writer. The existing gateway edge settings are still required as well:
`HEPHAESTUS_GATEWAY_DISPATCHER_LISTEN`, `HEPHAESTUS_GATEWAY_PUBLIC_AUTHORITY`,
`HEPHAESTUS_CADDY_CONFIGURATION_FILE`, and `HEPHAESTUS_CADDY_SERVER_NAME`.
A current generation host has the form
`g-<32 lowercase hexadecimal UUID>.<ui-namespace>` with the configured public
port when it is not 443. The reserved `/_heph/` namespace contains bootstrap
and is not release content. The bootstrap handoff uses a short-lived
fragment secret and exchanges it for the host-only secure UI session cookie;
release content must not receive the platform cookie or the handoff secret.

The UI handler rechecks the current generation, child session, release
permissions, artifact integrity, MIME, range/conditional behavior, and guest
response policy on the serving path. The kit and reference fixtures do not
replace those checks and must not be used as an authority boundary.

## Installed browser image

The installed-UI browser smoke uses the local tag
`localhost/hephestus-playwright:1.62.0-certutil`. Build it from the pinned
linux/amd64 Playwright 1.62.0 Noble base with:

```sh
scripts/installed-ui-browser-image/build.sh
```

The build requires Podman, a linux/amd64 host (or an amd64 emulation setup),
and registry access to `mcr.microsoft.com`; the script pulls the exact pinned
base before building so a fresh host does not depend on a preloaded image.

The Dockerfile pins base digest
`sha256:02bbb2155cd7109e3e9c741941097ed1608cf8b6fa44ee2595896da2bdc1f471`
and adds only `ca-certificates` and Ubuntu `libnss3-tools`. The successful
preflight image currently has digest
`sha256:9508609d5fe84e226585b7924928eff9afdbd1e0d06a4ca87cb62a0a3487e194`,
Chromium 151.0.7922.34, Node v24.18.0, and `certutil` available. Rebuilding
can produce a different derived digest when the package repository changes;
the base digest and image tag remain fixed by this recipe.

The browser runner creates a fresh NSS database for each disposable run and
imports only the Caddy fixture CA. Chromium then connects to the platform and
generation HTTPS origins without a certificate bypass or host trust-store
change. A disposable Caddy 2.10.2 preflight passed both origins with that
image. This proves the image and local TLS trust path; the full installed
release smoke still requires the Cooking fixture, daemon, gateway, RPC, and
browser environment and remains a separate acceptance run.

## Verification status

The repository has package checks, fixture build checks, local service checks,
and in-process daemon configuration/handler coverage. Installed Caddy/TLS,
PostgreSQL-to-HTTP, VM lifecycle, authenticated browser, restart, and
revocation acceptance remain separate pending work. Do not treat the local
fixture commands above as evidence for those installed-release properties.
