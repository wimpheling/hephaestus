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

`build.sh` compiles the three Python modules and stages them as one directory
artifact. The resulting `agent.toml` declares a required `runtime_git` session
repository capability scoped to `refs/heads/main`, the agent record/context
paths, and fast-forward updates only. The model request uses the existing
brokered-egress wire over the private broker-only vsock boundary.

The broker test uses a real local stream and checks the released wire and
sanitized response envelope against the existing Rust client shape. It does
not contact a model provider or claim VM-level model acceptance; that smoke
check belongs to the composed runtime journey.

This is release packaging and focused local evidence. It does not claim the
full production VM, browser, or GCP acceptance journey.
