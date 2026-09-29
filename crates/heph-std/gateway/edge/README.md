# Purpose

`gateway-edge` is the private gateway runtime around shared Caddy and bounded
HTTP dispatch. It converts already-authorized route state into a derived Caddy
configuration and sends validated requests to the selected VM or service
handler.

# Responsibilities

`LocalCaddyGatewayProvider` validates route bindings, rejects duplicate active
prefixes, applies only changed desired revisions, and reloads the complete
authoritative template during recovery. `GatewayDispatcher` selects the
longest exact route, enforces exposure and request limits, records accepted
invocations, and returns safe fallback responses for unavailable or malformed
requests. The Caddy administration port remains private to the host.

# When

Compose the provider with a private `CaddyAdministration`, a dispatcher, and a
complete configuration template. Call `reconcile` after a desired revision
changes and `recover` after restart, then pass normalized trusted requests to
`GatewayDispatcher` for bounded execution.
