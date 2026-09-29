# Code Surface

An experimental, local guide and code surface for structures already defined by
this repository. The Guide is the primary entry point: it renders the
co-located context READMEs and links each guide to the generated Cargo and
protobuf inventories. Crates and gRPC remain available as secondary exploratory
views.

## Run

From the repository root:

```sh
just code-surface
```

The recipe installs the pinned npm dependencies, regenerates the guide and both
code views, starts
the local server, and opens the browser. The equivalent commands from
`code-surface/` are:

```sh
npm ci
npm run dev
```

The generator needs Cargo and Buf. It uses `buf` from `PATH`, or the
repository's `.local/protobuf/bin/buf` when present. Run `npm run generate`
again after changing a manifest or `.proto` file while the server is open.

Open `#/guide` for the guide tree, `#/crates` for the workspace graph, or
`#/grpc` for the protobuf interface. The frontend uses vanilla JavaScript,
Cytoscape.js, and Vite. A short Node script is the only backend: it invokes the
existing code tools, validates the guide manifest against their inventories, and
writes JSON for the browser. There is no database or long-running API service.

`npm run check` validates the generated model and builds the static app. The
generated JSON and built app are local outputs and are not committed.

## Boundaries

The guide tree starts with authored READMEs and then reaches every workspace
crate directory from Cargo metadata, including nested crates and directories
without a README. A directory without a README remains navigable and is marked
with a warning so missing documentation is visible. The crate view covers
workspace packages and their declared workspace dependencies, including
optional and target-specific declarations. The gRPC view covers protobuf files,
services, RPC methods, messages, enums, and their type references. These views
describe declared structure, not the behavior inside functions or the runtime
state of an instance. The guide manifest lives in `scripts/guides.mjs`; its
source paths and explicit context/service references are checked during
generation. Unplaced generated contexts and services are reported in
`guides.json` and surfaced on guide pages.
