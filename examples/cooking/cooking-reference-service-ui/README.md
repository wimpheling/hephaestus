# Cooking managed reference UI service

This fixture is a small Hephaestus-managed HTTP UI service. It serves local
HTML and the versioned release UI kit CSS and optional theme/ready helper from
a long-lived, network-disabled Python process. It has no cookies, storage,
authority APIs, credentials, workspace mount, or state volume.

The service listens on `127.0.0.1:8080` and exposes `/readyz`, `/healthz`,
`/reference/index.html`, `/reference/heph-ui-kit-v1.0.0.css`, `/reference/heph-ui-kit-v1.0.0.js`, and a bounded
`/reference/identity` startup probe. The relative stylesheet link in the HTML
therefore stays inside the managed UI route.

`build.sh` verifies the vendored kit manifest and CSS/helper hashes before
copying the service, HTML, CSS, and helper into `bin/`. The vendored kit files
must match the canonical package under `web/assets/release_ui_kit`; the Node kit check covers
both the static and managed reference fixtures.

The `heph.gateways.toml` declaration uses the authenticated `http.service.v1`
contract. The `heph.ui.toml` declaration binds the managed service to the
exact gateway name, `/reference` route, and `index.html` entrypoint. Publication
and installed browser acceptance remain separate pipeline work.

For a local source smoke, build into a disposable absolute output directory,
start the copied `bin/reference-ui-service`, and request the readiness,
health, HTML, CSS, identity, and unmatched-path endpoints. The service uses
only files beside its executable and never reflects request headers.
