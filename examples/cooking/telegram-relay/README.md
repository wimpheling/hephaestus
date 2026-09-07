# External deterministic Telegram relay

`relay.py` runs outside Hephaestus. It implements the bounded authenticated
`POST /v1/messages` contract used by the cooking guest with durable SQLite
idempotency. Run with `python3 relay.py --database /absolute/test.sqlite3
--credential-file /absolute/external-credential --port 8091`. The credential file
is external deployment data; never commit it or put its bytes in arguments.
This fixture listens only on loopback, disables request logs and uses a fake
transport which makes no network calls and requires no Bot API token.

Requests contain only `idempotency_key`, `user_id` (alice/bob), and bounded `text`.
Identical retries return the durable original redacted result; conflicting key
reuse returns 409. Authentication failure returns 401 and malformed input 400.
The fake delivery and ledger commit are one transaction. Real Telegram transport,
accounts, Bot API tokens and live smoke tests are outside MVP-05 scope. The
deterministic relay exercises Hephaestus outbound capabilities and recovery
without a provider account or network delivery.

Conformance tests live in sibling `cooking-agent/test_cooking.py` and include
authentication, limits, conflict and a crash after delivery before client receipt.
