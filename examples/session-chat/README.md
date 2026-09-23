# Reference session-chat release

This directory contains the release-owned session-chat protocol, ordinary Git
adapter, and reference agent package for MVP-06. The platform supplies only
the authorized repository capability, runtime context, and brokered model
egress; transcript interpretation, response correlation, batching, and model
context remain release-owned.

The reference protocol requires an operation warning before a release-owned
command or adapter exposes a fork or tombstone. A fork carries reachable
records and tombstones into a new session, but requires fresh target authority
and model binding; source-session authority is not inherited from Git history.
A tombstone hides a record from the ordinary view while retaining the record
and tombstone in Git history, so it is not erasure. These are reference-release
semantics, not a Hephaestus core approval flow or a requirement for a
particular UI control; the command or adapter presenting the operation must
explain them before acting.

Run the focused offline suite with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s examples/session-chat -p 'test_*.py' -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s examples/session-chat/tests -p 'test_*.py' -v
```

The second discovery root includes the test-only guest denial probe, which is
kept outside the release package and is therefore not part of the first root's
module discovery.

The denial probe requires three distinct canonical UUIDs:
`--target-repository-id`, `--source-repository-id`, and
`--other-repository-id`. The runtime fixture supplies those IDs while keeping
the control, secret, and Git-helper paths at their production guest locations.
The released-VM denial probe is opt-in with
`HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E=1`. On a prepared Fedora host
as described in [`docs/vm-libkrun.md`](../../docs/vm-libkrun.md), the verified
standalone invocation is:

```sh
env -u HEPHAESTUS_APP_COOKING_E2E \
  HEPHAESTUS_APP_SESSION_CHAT_E2E=1 \
  HEPHAESTUS_APP_LIBKRUN_E2E=1 \
  HEPHAESTUS_APP_COOKING_BUILD_PROOF=1 \
  HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E=1 \
  scripts/run-libkrun-integration.sh
```

The default deterministic session run does not enable the denial probe.

The browser-backed lifecycle is separately opt-in and runs in order as
browser-initial, restart/recovery, concurrency, and fork. The later phases
reuse the installed UI and source session through restart/recovery and
concurrency; the fork phase creates a fresh target repository, authority, and
UI installation while checking inherited history. Each phase has its own
typed report, and a missing or failed phase does not count as acceptance.
Prepare
the KVM, Podman, Rust musl target, digest-pinned guest images, and browser
dependencies using [`examples/cooking/README.md`](../cooking/README.md) and
[`docs/vm-libkrun.md`](../../docs/vm-libkrun.md). The shared runner provisions
the OIDC/Caddy bridge and installed-UI fixture; choose a free Caddy port and
keep the HTTPS origin equal to that port:

```sh
CADDY_PORT=4443
env -u HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E \
  HEPHAESTUS_COOKING_SCENARIO=session-chat \
  HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE=1 \
  HEPHAESTUS_COOKING_BROWSER_E2E=1 \
  HEPHAESTUS_CADDY_TEST_TLS=1 \
  HEPHAESTUS_CADDY_TEST_PUBLIC_PORT="$CADDY_PORT" \
  HEPHAESTUS_PLATFORM_HTTPS_ORIGIN="https://platform.localhost:${CADDY_PORT}" \
  HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E=1 \
  HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E=1 \
  HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E=1 \
  HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE='...@sha256:<python-image-digest>' \
  HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE='...@sha256:<rust-builder-image-digest>' \
  examples/cooking/run.sh
```

The two image values must be exact compatible digests prepared as described in
the linked Cooking setup; the placeholders above are not runnable values. This
command exercises the local production path only. The corresponding GCP
projector uses the fixed phase IDs `browser-initial`, `browser-recovery`,
`browser-concurrency`, and `browser-fork`, with session reports
`session_chat_new`, `session_chat_ui`, `session_chat_concurrent`, and
`session_chat_fork`. The implementation and typed gate are present, but real
runtime, browser, recovery, concurrency, fork, and GCP acceptance evidence
remain pending.

The combined lifecycle can additionally set
`HEPHAESTUS_APP_SESSION_CHAT_NEGATIVE_E2E=1` alongside the browser, restart,
concurrency, and fork flags. The runner starts a second fresh guest process
for the negative summary and supplies its denial-probe flag internally; the
combined path still requires all four browser reports. This second process
runs one exact denial test and writes `session-chat-negative-summary.json`.
Its private raw log is mode `0600`; collection retains the bounded summary.

The separate guest negative process is enabled with
`HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E=1` in the standalone invocation
above. Keep the browser/restart/concurrency/fork flags out of that direct
invocation. It exercises the probe within the ordinary golden suite and emits
the ten-check host result; it does not use the combined path's separate exact
test and summary step. Fresh combined runtime and GCP evidence remain pending.

Do not install UI dependencies inside the release fixture before a VM build.
The generated `ui/node_modules` tree can contain symlinks and must be created
in an external temporary directory, or removed before source materialization.

`build.sh` compiles the three Python modules and stages them as one directory
artifact. The resulting `agent.toml` declares a required `runtime_git` session
repository capability scoped to `refs/heads/main`, the agent record/context
paths, and fast-forward updates only. The model request uses the existing
brokered-egress wire over the private broker-only vsock boundary.

The broker test uses a real local stream and checks the released wire and
sanitized response envelope against the existing Rust client shape. The
released-VM probe additionally proves the authorized model control, source and
other-repository denials, prohibited-path denial, and undeclared model
destination/rule denials without exposing credentials or raw diagnostics.

This is release packaging and focused local evidence. It does not claim the
full production VM, browser, or GCP acceptance journey.
