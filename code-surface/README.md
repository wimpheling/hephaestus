# Code Surface

An experimental, local view of two structures already defined by this repository:
the Cargo workspace and the protobuf/gRPC schema. It generates data from Cargo
metadata and Buf descriptors when the app starts. The browser app reads those
generated files; it does not maintain a second crate or RPC inventory.

## Run

From the repository root:

```sh
just code-surface
```

The recipe installs the pinned npm dependencies, regenerates both views, starts
the local server, and opens the browser. The equivalent commands from
`code-surface/` are:

```sh
npm ci
npm run dev
```

The generator needs Cargo and Buf. It uses `buf` from `PATH`, or the
repository's `.local/protobuf/bin/buf` when present. Run `npm run generate`
again after changing a manifest or `.proto` file while the server is open.

Open `#/crates` for the workspace graph or `#/grpc` for the protobuf interface.
The frontend uses vanilla JavaScript, Cytoscape.js, and Vite. A short Node
script is the only backend: it invokes the existing code tools and writes JSON
for the browser. There is no database or long-running API service.

`npm run check` validates the generated model and builds the static app. The
generated JSON and built app are local outputs and are not committed.

## Boundaries

This first surface covers workspace packages and their declared workspace
dependencies, including optional and target-specific declarations. The gRPC
view covers protobuf files, services, RPC methods, messages, enums, and their
type references. It describes declared structure, not the behavior inside
functions or the runtime state of an instance. It can grow by adding more
code-derived surfaces without requiring application code to fit a new DSL.
