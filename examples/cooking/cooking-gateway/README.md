# Cooking gateway

The Rust executable reads one CBOR http.v1 request and emits one bounded response.
POST `/gateway/cooking/telegram` retains that full path in the private request.
The route manifest declares `/cooking/telegram` under the shared namespace. Parameters
`inbound_placeholder`, `alice_provider_id` (1001), `bob_provider_id` (1002) are read
from the sealed runtime parameters file. Exactly one verification header must
equal the host-generated non-secret placeholder; raw verification credentials
never enter the guest. The provider IDs must be distinct.

Valid requests return 200 and select one publication to `cooking_requests` with
route `/telegram/updates`, kind `cooking.telegram.received.v1`, and JSON containing
only `provider_update_id`, `user_id`, `command` (recipe), and bounded trimmed
`text`. Bodies are at most 16384 bytes, text 2048 bytes, and update IDs nonnegative
signed 64-bit integers. Missing/invalid verification returns 401, unknown user
403, and malformed/oversized input 400. No raw bodies or headers are projected.

The application selects stable key `telegram-update-<update_id>`. The host's
existing atomic publication transaction persists at most one event per bound
producer/key, including retries carrying different bodies for the same update.
This state-free one-shot gateway does not claim its own durable ledger: it uses
the generic publication idempotency contract. Cooking independently deduplicates
mailbox delivery. No cooking state, repository or outbound API authority is used.

`build.sh` builds locked vendored Rust offline and exports only the executable.
`agent.toml` and `heph.gateways.toml` declare runtime policy and the sole symbolic
mailbox slot; installation supplies concrete authority. Verify formatting,
Clippy, tests and docs with Cargo. Installed full-journey evidence is separate
from these application tests; no real Telegram transport is enabled here.
