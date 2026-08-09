# Persistent gateway service runtime and development workflow

Owner: unassigned

## Outcome

Let a gateway release run a normal long-lived web server inside a guest and
serve it through the existing shared Caddy deployment. This is the natural
development and production model for applications that already speak HTTP,
while retaining Hephaestus ownership of routing, authority, isolation,
lifecycle, and audit.

It is intentionally separate from MVP 03's stateless one-request gateway
handler. MVP 03 starts a fresh short-lived VM for one bounded request and
response; it needs a small stdin/stdout handler ABI, not a guest HTTP listener.

## Why this is a separate task

A persistent server changes fundamental runtime semantics:

- readiness and health checks;
- listener protocol and port ownership inside the guest;
- connection lifecycle, draining, cancellation, and process supervision;
- revision cutover and rollback while requests are in flight;
- scale, concurrency, idling, and crash recovery;
- local development parity, logs, and debugging.

Those concerns should not be hidden inside the synchronous, bounded MVP 03
contract or accidentally turn its microVM request path into a long-lived
service runtime.

## Initial direction

Use an explicitly declared service handler mode, distinct from `http.v1`
one-request handlers. Hephaestus starts the released command once, and the
command binds only a guest-private listener on a platform-provided transport.
The host-side gateway adapter forwards normalized requests from the shared
Caddy deployment over an authenticated private channel. Released code never
receives Caddy administration, a public listener, or unrestricted host
networking authority.

The exact guest transport (vsock, a private Unix socket bridge, or another
bounded mechanism) must be selected with its framing, flow-control, timeout,
and backpressure properties. It must not fall back to arbitrary guest TCP
exposure.

## Design checklist

- [ ] Define service-mode release configuration, compatibility with the
  stateless handler mode, and migration/upgrade semantics.
- [ ] Define guest listener ownership, authenticated host-to-guest transport,
  readiness/health contract, request limits, connection limits, and response
  semantics.
- [ ] Define Caddy-to-service routing, load balancing, revision cutover,
  draining, idling, restart, rollback, and failure behavior.
- [ ] Define gateway-session authority for persistent connections and the
  relationship between a request, a service instance, and a release revision.
- [ ] Define development workflow: local run, live reload expectations, logs,
  debugger access, port inspection, and production-parity boundaries.
- [ ] Define the security model for host headers, client identity, TLS
  termination, internal endpoints, SSRF, WebSockets/streaming, and secret
  substitution.
- [ ] Implement real Caddy, guest-runtime, cutover, crash, concurrency, and
  adversarial protocol tests before exposing service mode.

## Non-goals

This task does not replace MVP 03's bounded stateless invocation mode. It does
not grant guest Caddy admin access, public host ports, arbitrary listener
binding, or ambient project/network authority.

## Related work

- [MVP 03: Gateway HTTP routing and invocation](mvp-03-event-ingress-and-caddy-routing.md)
- [MVP 03.1: Gateway principals and authority](mvp-03.1-gateway-principals-and-authority.md)
