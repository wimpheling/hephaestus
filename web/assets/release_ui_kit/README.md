# Hephaestus release UI kit

Version 1.0.0 is a standalone CSS-only kit for immutable release UIs. It
contains the existing Hephaestus semantic token source and a small prefixed
component layer for containers, layout, type, panels, buttons, status labels,
and inputs.

The kit has no Phoenix, LiveView, HEEx, Tailwind runtime, authentication,
session, navigation, or platform-authority behavior. It is suitable for a
release UI rendered on its isolated origin. Its component classes use the
`heph-ui-` prefix to avoid class-name collisions within that document.
The kit follows the operating system dark preference when the document root
does not have a valid theme attribute. An explicit `data-theme="light"` or
`data-theme="dark"` always overrides that preference:

```html
<html lang="en">
```

Use `data-theme="light"` or `data-theme="dark"` when a release needs a fixed
palette. The kit does not persist or change this value.

Build and test it without downloading dependencies:

```sh
cd web/assets/release_ui_kit
npm run test
npm run check
```

The build reads the exact token source at
`../css/design_system/tokens.css`, combines it with `src/components.css`, and
writes the derived files below `dist/`:

```text
dist/heph-ui-kit-v1.0.0.css
dist/manifest.json
```

`manifest.json` records the package version and SHA-256 digests for the final
CSS, token source, and component source. `dist/` is ignored because it is
generated output, matching the repository's existing Phoenix asset policy.
Run `npm run test` in a checkout to rebuild and validate the output. Run
`npm run check` in CI or before packaging to compare the existing `dist/` bytes
with a fresh render and fail on missing or stale output. `npm pack` invokes the
same build through `prepack`, and the test suite exercises an offline
`npm pack --dry-run` file-list check; no registry publication is performed.
Consumers use the generated `dist` files and do not need the repository token
source.

Plain HTML can consume the generated CSS directly:

```html
<link rel="stylesheet" href="/ui/heph-ui-kit-v1.0.0.css">

<body class="heph-ui-kit">
  <main>
    <div class="heph-ui-container heph-ui-stack">
      <span class="heph-ui-status">Ready</span>
      <section class="heph-ui-panel heph-ui-stack">
        <h1 class="heph-ui-title">Release UI</h1>
        <p class="heph-ui-text">A static release-owned surface.</p>
        <a class="heph-ui-button heph-ui-button--primary" href="/continue">Continue</a>
      </section>
    </div>
  </main>
</body>
```

For a release, copy the generated CSS into the build output and declare it as
an immutable `text/css` artifact. A `heph.ui.toml` static declaration should
reference that exact release-relative artifact path and use an HTML artifact
as its `text/html` entrypoint. The release resolver binds both paths to exact
artifact IDs; the UI does not read Git or platform state at runtime.

A Cooking release can receive this pinned distribution as source/build input
and copy it into `/workspace/output`. Static UIs declare the CSS as a release
artifact; managed UI services can serve the same CSS from their declared
content route. MIME declarations must remain separate for HTML and CSS files;
a directory artifact uses one declaration MIME for all expanded files.

The Node checks validate deterministic bytes, token-source inclusion, manifest
hashes, version/export naming, absence of Phoenix/Tailwind imports, and basic
class coverage. They do not replace browser rendering, accessibility, or
visual regression checks.
