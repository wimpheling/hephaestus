# Inspecting a cooking request

Use the authenticated inspection APIs with an audience-bound mediator token for
each RPC. `GatewayService` ingress/mailbox provenance resolves the public route,
gateway revision, published mailbox event and delivery run ID. The instance
inspection APIs resolve the state volume, fenced lease and dispatch disposition.

Call `/hephaestus.run.v1.RunService/GetRun` with
`{"runId":{"value":"RUN_UUID"}}` to inspect the cooking release/revision,
exact input commit/ref, result/proposal and final run state. Mailbox delivery
runs resolve their immutable delivery target and attachment even though they
have no Git-triggered `run_requests` row.

Call `/hephaestus.run.v1.RunService/GetRunProvenance` with
`{"runId":{"value":"RUN_UUID"},"page":{"pageSize":50}}` for the immutable
authorization snapshot ID, model version and normalized hash, plus HTTPS audit
records. Pass the returned `nextPageToken` as `page.pageToken` until empty.
HTTPS records have stable ID order and correlate authorization decisions and
substitution outcomes by request ID. They expose exact lease, binding, rule and
secret-version IDs; they do not expose destinations, headers, bodies or values.

Run read permission is required for the base response. Each HTTPS record also
requires `inspect_metadata` on its secret. An empty HTTPS page therefore means
no *visible* records, not necessarily no calls. Authorization and use records
are historical evidence, not current permission to execute. Rotating/revoking
a credential does not remove earlier evidence from an authorized inspector.

The broker commits an authorization decision before an HTTPS operation and
records the adapter outcome afterward under the same request ID. An interrupted
call may have a decision without an outcome; this is not proof that no external
effect occurred. Failed adapters and oversized responses have failed outcomes.
Only attempts attributable to an exact authorized lease/rule snapshot enter this
view. Invalid credentials, unknown rules and unbound slots must not be mistaken
for use of some other valid binding. Current authority is rechecked before a
successful external response can be delivered to the guest.

The PostgreSQL secret lifecycle integration test exercises the restricted
application role, exact historical versions after rotation/revocation,
pagination, a run reader without secret inspection, and an unrelated owner:

```sh
HEPHAESTUS_POSTGRES_TEST_URL=postgres://... cargo test -p secret-postgres --test postgres --all-features
```

The complete MVP-05 crash, update, real Telegram and VM acceptance scenarios
remain separate from this inspection API check.
