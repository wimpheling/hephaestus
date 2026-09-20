# Cooking managed reference UI service

This fixture is a small Hephaestus-managed HTTP UI service. It serves local
HTML and the versioned release UI kit CSS and optional theme/ready helper from
a long-lived, network-disabled Python process. It has no persistent storage,
authority APIs, credentials, workspace mount, or state volume. The fixed
response-policy probes below intentionally emit test-only response headers and
do not persist cookies.

The service listens on `127.0.0.1:8080` by default (the local probe may set
`HEPHAESTUS_REFERENCE_SERVICE_PORT`) and exposes `/readyz`, `/healthz`,
`/reference`, `/reference/`, `/reference/index.html`,
`/reference/heph-ui-kit-v1.0.0.css`, `/reference/heph-ui-kit-v1.0.0.js`, and a
bounded `/reference/identity` startup probe plus four admission probes:
`/reference/header-policy`, `/reference/probe-set-cookie`,
`/reference/probe-location`, and `/reference/probe-refresh`. The same declared
documents, assets, identity probe, and admission probes are also available
under the exact private gateway paths `/gateway/reference/...`; these are
explicit aliases for the gateway transport and do not enable arbitrary prefix
rewriting. The relative stylesheet and helper links in the HTML therefore stay
inside the managed UI route.
The executable uses the pinned image's `/usr/local/bin/python3` path because
isolated guest commands start with a cleared environment and no inherited
`PATH`.

`build.sh` verifies the vendored kit manifest and CSS/helper hashes before
copying the service, HTML, CSS, and helper into `bin/`. The vendored kit files
must match the canonical package under `web/assets/release_ui_kit`; the Node kit check covers
both the static and managed reference fixtures.

The `heph.gateways.toml` declaration uses the authenticated `http.service.v1`
contract. The `heph.ui.toml` declaration binds the managed service to the
exact gateway name, `/reference` route, `index.html` entrypoint, iframe
presentation, and `ui_kit_version = 1`. It declares five GET APIs on that
gateway. The `header-policy` response contains only the booleans
`authorization_absent`, `cookie_absent`, `forwarded_absent`, and
`x_forwarded_absent`; it never echoes header values. The three response-policy
probes deliberately return fixed `Set-Cookie`, `Location`, and `Refresh`
headers so the edge can verify guest-header stripping and response rejection.
The identity API returns only the service's JSON `pid` and `startup_id` fields;
`startup_id` changes when the service restarts. Its `no_store` policy is
explicit.
Publication and installed browser evidence are recorded in the [release UI
acceptance record](../../../tasks/in-progress/release-owned-distribution-ui-surfaces.md#final-installed-ui-runtime-proof-2026-09-21).
The local service smoke below remains separate from the installed runtime
proof. Run 32 covered owner navigation, guest-boundary behavior, live child
replacement, disable/reactivation/removal, parent-session revocation, and
audit markers; the final repository-wide quality gate remains pending.

For a local source smoke, build into a disposable absolute output directory,
start the copied `bin/reference-ui-service`, and request the readiness,
health, HTML, CSS, helper, identity, and unmatched-path endpoints:

```sh
managed_output=$(mktemp -d)
HEPHAESTUS_REFERENCE_SERVICE_UI_OUTPUT="$managed_output" \
  examples/cooking/cooking-reference-service-ui/build.sh
(cd "$managed_output/bin" && exec python3 ./reference-ui-service) &
service_pid=$!
trap 'kill "$service_pid" 2>/dev/null || true; rm -rf "$managed_output"' EXIT
curl --fail http://127.0.0.1:8080/readyz
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/reference/index.html
curl --fail http://127.0.0.1:8080/reference/heph-ui-kit-v1.0.0.js
curl --fail http://127.0.0.1:8080/gateway/reference/index.html
curl --fail http://127.0.0.1:8080/gateway/reference/heph-ui-kit-v1.0.0.css
curl --fail http://127.0.0.1:8080/gateway/reference/heph-ui-kit-v1.0.0.js
curl --fail http://127.0.0.1:8080/gateway/reference/identity
curl --fail http://127.0.0.1:8080/gateway/reference/header-policy
curl --fail http://127.0.0.1:8080/gateway/reference/probe-set-cookie
curl --fail http://127.0.0.1:8080/gateway/reference/probe-location
curl --fail http://127.0.0.1:8080/gateway/reference/probe-refresh
test "$(curl -s -o /dev/null -w '%{http_code}' \
  http://127.0.0.1:8080/gateway/reference/unknown)" = 404
```

The service uses only files beside its executable and never reflects request
headers. Its network-disabled behavior applies to the release-agent fixture;
this local Python process only binds to loopback. The smoke is not proof of
publication through the authenticated gateway, installed Caddy/TLS, VM
lifecycle, or browser authorization. Those boundaries are covered by the
completed bounded runtime evidence; the live child guest replacement proof is
distinct from the separated daemon restart proof, and no account-administration
API or browser-driven account-admin action is claimed. See
[`docs/release-ui.md`](../../../docs/release-ui.md) for the daemon UI-origin
settings and serving authority.
