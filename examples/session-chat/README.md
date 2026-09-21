# Reference session-chat release

This directory contains the release-owned session-chat protocol, ordinary Git
adapter, and reference agent package for MVP-06. The platform supplies only
the authorized repository capability, runtime context, and brokered model
egress; transcript interpretation, response correlation, batching, and model
context remain release-owned.

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
