# Purpose

`brokered-egress-client` lets a released guest ask the host to perform one
approved secret-backed operation. It carries the run, symbolic slot,
destination, operation, body, and opaque credential over a bounded framed
stream, then turns the host reply into a typed result. The client has no host
transport or substitution implementation; response safety depends on the host
broker's sanitization contract.

# Responsibilities

The wire request carries the run, symbolic slot, exact destination, semantic
operation, body, and opaque credential that the host needs to match the call
to its lease. The response carries only a bounded, host-sanitized body and one
of `Succeeded`, `Denied`, or `Retryable`; upstream headers and credential
material are excluded by the host broker's response contract.

`BrokeredHttpsClient::call` enforces `MAX_FRAME_BYTES` in both directions and
rejects malformed or empty frames before they reach the broker. The host still
performs the authority check; this client supplies the bounded transport used
after that workflow has selected a broker request.

# When

Use it in a released guest process with the already connected broker stream
when a secret should be injected into an approved outbound operation:

```rust
let mut client = BrokeredHttpsClient::new(stream);
let response = client.call(&WireBrokerRequest {
    credential, run_id, slot, destination, operation, body,
})?;
```
