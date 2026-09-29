# Purpose

`gateway-domain` defines canonical route, service, and bounded HTTP contracts
for the platform edge. It gives configuration reconciliation, UI-origin
handlers, and service workers one vocabulary for route bindings, revisions,
targets, ownership, probes, logs, mailbox publication, and invocation outcomes.

# Responsibilities

The domain validates route paths, methods, body limits, service identities,
ownership leases, target revisions, and lifecycle transitions. UI admission
checks the child session, actor, installation generation, canonical path, and
selected authenticated route before producing a dispatcher request; it strips
credential-bearing browser headers and rejects guest attempts to set browser
cookies. Service contracts preserve ownership and bounded log state so a stale
worker cannot launch or publish through a newer revision.

# When

Use this crate at the edge or in a gateway adapter when an already-authorized
request must enter the dispatcher:

```rust
let gateway_request = prepare_gateway_request(&request, &admission)?;
```

Validate the provider's route binding and authority again at the durable
boundary before executing the selected service.
