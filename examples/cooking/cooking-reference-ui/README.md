# Cooking release reference UI

This fixture is a small static release UI. It has no JavaScript, external
resources, chat surface, or platform credentials. Its build and guest network
profiles are disabled.

The CSS under `vendor/release-ui-kit/v1.0.0/dist` is generated from the
canonical `web/assets/release_ui_kit` package with `npm run build`. Its
manifest records the canonical token/component source hashes and the build
script verifies those hashes before copying the CSS into the release output.

The fixture is intended for the existing Cooking Git/build/release path. Its
published-service integration is pending; it is not a direct database or
artifact-store fixture.
