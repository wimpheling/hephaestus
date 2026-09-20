# Cooking release reference UI

This fixture is a small static release UI. Its only JavaScript is the optional
versioned kit helper for validated theme messages and iframe-ready signaling;
it has no cookies, storage, chat surface, or platform credentials. Its build
and guest network profiles are disabled.

The CSS under `vendor/release-ui-kit/v1.0.0/dist` is generated from the
canonical `web/assets/release_ui_kit` package with `npm run build`. Its
manifest records the canonical token/component/helper hashes and the build
script verifies those hashes before copying the CSS and helper into the release
output.

The fixture is intended for the existing Cooking Git/build/release path. Its
published-service integration is pending; it is not a direct database or
artifact-store fixture. The release declaration contains the same full-page
static UI for project, repository, and organization-global owners, with route
bases `reference`, `reference-repository`, and `reference-global`. Each uses
`index.html`, `ui_kit_version = 1`, and `no_store` caching. The CSS and helper
are separate `text/css` and `text/javascript` artifacts.

Build it into a disposable absolute directory from the repository root:

```sh
static_output=$(mktemp -d)
HEPHAESTUS_REFERENCE_UI_OUTPUT="$static_output" \
  examples/cooking/cooking-reference-ui/build.sh
"$static_output/bin/reference-ui-check"
```

The build verifies the vendored manifest and source hashes before materializing
`dist/index.html` and the versioned kit assets. This local check does not prove
publication through Caddy, an authenticated session, or installed browser
serving. See [`docs/release-ui.md`](../../../docs/release-ui.md) for the
package upgrade and daemon-origin boundary.
