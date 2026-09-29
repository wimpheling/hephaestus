# Purpose

`oci-builder-worker` runs the durable OCI production and rootfs materialization
loops. It combines a source checkout provider, isolated build engine, registry
token issuer, output publisher, and rootfs exporter into the worker behavior
expected by `heph-build`.

# Responsibilities

`OciImageProductionWorker::run_once` claims one job, checks the Dockerfile and
source policy, prepares the checkout, executes the isolated build, publishes
the immutable image, and records the output or a bounded failure. Network and
ambient credentials are disabled in the `IsolatedOciBuild` contract, and the
worker passes only approved source, base, and output paths to its engine.

`RootfsMaterializationWorker` claims verified image output, installs the
trusted guest init, validates and seals the root tree, writes the daemon
manifest, and records the materialized root. Cleanup and recovery are tied to
durable job state so a restarted worker can continue safely.

# When

Create the production worker with `OciImageProductionWorker::new`, call
`run_once` from the build loop, and run `RootfsMaterializationWorker::run_once`
for queued immutable outputs. Use `write_manifest` after materialization when
the daemon needs to rebuild its root index.
