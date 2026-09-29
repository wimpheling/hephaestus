# Platform core

Platform owns the cross-domain boundary through which an installation is
addressed and invoked. Its logical scope includes gateway contracts, route
declarations and resolution, bounded invocation, transport contracts, and
shared mechanics that connect Forge, Image, Runtime, and the control plane.

## Current contracts

- [`agent-config/`](agent-config/) parses and validates versioned repository
  configuration, including image, build, guest, capability, gateway, UI,
  parameter, secret-slot, and result declarations. It also produces stable
  configuration and build-definition hashes.
- [`rpc-proto/`](rpc-proto/) contains generated protobuf and Connect transport
  types. Generated types are converted at the transport boundary rather than
  becoming domain models.
- [`runtime-types/`](runtime-types/) provides stable IDs shared across runtime
  domains.

The gateway contracts are still in their dedicated physical root:
[`gateway/`](../gateway/) and its PostgreSQL or transport adapters. They are
documented here as part of the platform ownership boundary without implying
that those crates have moved. Identity, authorization, and secret contracts
belong to the [`Auth`](../auth/) boundary. Events, control-plane persistence,
and mailbox storage similarly retain their declared package contexts.

Platform routes are distinct from repository-declared gateway routes. A gateway
receives only a bounded request after exact route and revision resolution,
identity checks, and invocation authorization. The gateway edge is not a
generic privileged proxy, and project content does not gain management-plane
authority by crossing the invocation boundary.

## Implementations and composition

Gateway reconciliation and dispatch are in [`heph-std/gateway/edge/`](../../heph-std/gateway/edge/),
and the daemon wires these boundaries in [`heph-app`](../../heph-app/). The
platform core owns the vocabulary and checks; it does not own a listener,
Caddy process, database pool, or VM provider.

See [`docs/authorization.md`](../../../docs/authorization.md),
[`docs/persistent-gateway-services.md`](../../../docs/persistent-gateway-services.md),
[`docs/secrets.md`](../../../docs/secrets.md), and
[`docs/application.md`](../../../docs/application.md).
