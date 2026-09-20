# Cooking managed reference UI service

This fixture is a small Hephaestus-managed HTTP UI service. It serves local
HTML and the versioned release UI kit CSS and optional theme/ready helper from
a long-lived, network-disabled Python process. It has no cookies, storage,
authority APIs, credentials, workspace mount, or state volume.

The service listens on `127.0.0.1:8080` and exposes `/readyz`, `/healthz`,
`/reference`, `/reference/`, `/reference/index.html`,
`/reference/heph-ui-kit-v1.0.0.css`, `/reference/heph-ui-kit-v1.0.0.js`, and a
bounded `/reference/identity` startup probe. The relative stylesheet and
helper links in the HTML therefore stay inside the managed UI route.

`build.sh` verifies the vendored kit manifest and CSS/helper hashes before
copying the service, HTML, CSS, and helper into `bin/`. The vendored kit files
must match the canonical package under `web/assets/release_ui_kit`; the Node kit check covers
both the static and managed reference fixtures.

The `heph.gateways.toml` declaration uses the authenticated `http.service.v1`
contract. The `heph.ui.toml` declaration binds the managed service to the
exact gateway name, `/reference` route, `index.html` entrypoint, iframe
presentation, and `ui_kit_version = 1`. It also declares the `identity` GET
API at `/reference/identity` on that gateway. An authorized managed UI request
receives only the service's JSON `pid` and `startup_id` fields; `startup_id`
changes when the service restarts. Its `no_store` policy is explicit.
Publication and installed browser acceptance remain separate pipeline work.

For a local source smoke, build into a disposable absolute output directory,
start the copied `bin/reference-ui-service`, and request the readiness,
health, HTML, CSS, helper, identity, and unmatched-path endpoints:

```sh
managed_output=$(mktemp -d)
HEPHAESTUS_REFERENCE_SERVICE_UI_OUTPUT="$managed_output" \
  examples/cooking/cooking-reference-service-ui/build.sh
(cd "$managed_output/bin" && exec ./reference-ui-service) &
service_pid=$!
trap 'kill "$service_pid" 2>/dev/null || true; rm -rf "$managed_output"' EXIT
curl --fail http://127.0.0.1:8080/readyz
curl --fail http://127.0.0.1:8080/reference/index.html
curl --fail http://127.0.0.1:8080/reference/heph-ui-kit-v1.0.0.js
```

The service uses only files beside its executable and never reflects request
headers. Its network-disabled behavior applies to the release-agent fixture;
this local Python process only binds to loopback. The smoke is not proof of
publication through the authenticated gateway, installed Caddy/TLS, VM
lifecycle, or browser authorization. See
[`docs/release-ui.md`](../../../docs/release-ui.md) for the daemon UI-origin
settings and serving authority.
