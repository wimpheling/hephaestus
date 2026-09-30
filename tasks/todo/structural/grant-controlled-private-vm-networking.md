# Grant-controlled private VM networking — design draft

Owner: unassigned

## Purpose

Explore a private networking contract through which VM A exposes named TCP or
UDP endpoints and VM B connects only under an explicitly bounded grant. An
ordinary PostgreSQL client in B should be able to connect to
`postgresql-XXX:5432` without a Heph-specific client or protocol. Applications
using QUIC can use a UDP endpoint; TCP, UDP, and QUIC are protocols, not backend
libraries.

This is a design and exploration TODO. It records the desired contract and
candidates for evaluation; it does not approve a backend, start implementation,
or assign roadmap priority. A rootless implementation is worth investigating
because the present VM host contract avoids privileged network changes. Whether
rootless mediation can satisfy the entire contract, or a deliberately
provisioned privileged backend is needed, remains open.

## Current repository context

The [libkrun host contract](../../../docs/vm-libkrun.md) runs one unprivileged
passt process per networked VM. Its VM-facing NIC uses a per-VM Unix stream
socket, without creating host network interfaces or changing host routes or
firewall rules. The present provider supplies TCP and UDP egress and DHCP/DNS,
with explicitly declared TCP ingress forwards on host `127.0.0.1`.
The [passt launcher](../../../crates/heph-std/runtime/vm/libkrun/src/network.rs)
sets `--udp-ports none`, disabling UDP ingress. This is attachment and forwarding
machinery, not the proposed private endpoint grant enforcement.

The [portable VM specification](../../../crates/heph-core/runtime/vm/trait/src/vm_trait/spec.rs)
currently distinguishes disabled, broker-only, and user-mode networking. Its
ingress protocol enum contains TCP only. Its separate private HTTP service
specification targets a guest loopback TCP port. The
[private service connection](../../../crates/heph-core/runtime/vm/trait/src/vm_trait/http.rs)
is a full-duplex byte stream, while the
[gateway exchange](../../../crates/heph-std/gateway/edge/src/service_http/exchange.rs)
uses that stream for bounded HTTP/1 requests. The current public-service contract
is HTTP-only; the raw stream is a possible integration point to investigate,
not an existing general TCP/UDP network.

The [authorization documentation](../../../docs/authorization.md) describes
the production identity, transaction-bound authorization, and audit authority.
The adjacent
[project-defined permission systems and delegated authorization task](project-defined-permission-systems-and-delegated-authorization.md)
explores application-owned permissions. Private endpoint connectivity concerns
Heph's network boundary. PostgreSQL credentials and database roles, and other
application permissions, remain separate from permission to reach an endpoint.

## Contract to preserve during exploration

| Decision | Meaning |
| --- | --- |
| Named endpoint | A declares a stable endpoint identity, owning workload/service, protocol, and permitted destination port. A name such as `postgresql-XXX` is a client-facing reference to that identity. |
| Bounded connect grant | B receives authority to connect to specific endpoint identities, protocols, and ports for a bounded lifetime and scope. Delegation cannot exceed the delegator's authority. |
| Ordinary clients | Guest applications use ordinary TCP/UDP sockets and service names. PostgreSQL keeps its normal wire protocol and its own authentication. |
| Trusted source attachment | Heph derives source workload/session identity from the provider-controlled attachment. Guest IP, MAC, DNS, headers, and claimed identity are untrusted inputs. |
| Complete mediation | Knowing an endpoint's private IP or port cannot confer access. Every available path to the protected endpoint is subject to the same decision, including IPv4 and IPv6. |
| Default denial | A reachable attachment or successful name lookup does not authorize a connection. Unlisted endpoint/protocol/port combinations are denied. |
| Portable domain | Core owns workload identity, endpoint identity, grants, lifecycle semantics, and provider-neutral networking contracts. Standard adapters implement attachment, enforcement, resolution, and transport. |

DNS makes an authorized service convenient to address. DNS visibility, obscured
addresses, or a proxy reachable by every VM cannot serve as the access control
boundary. A grant must be enforced where Heph can attribute the source reliably
and prevent an alternative route to A.

A useful conceptual path is:

```text
B's ordinary socket
  → provider-controlled attachment identifying B's exact runtime session
  → endpoint/protocol/port grant enforcement
  → private transport (authenticated between hosts when needed)
  → A's declared endpoint
```

This describes responsibilities rather than a selected topology. A user-mode
forwarder might enforce them at its guest-facing socket and trusted host
connect operation. A kernel backend might enforce them at a controlled packet
boundary. Either needs an account of every remaining path and every identity
translation.

Private Ethernet switching between guests can use each guest's existing
TCP/UDP stack. It does not inherently require a new host user-mode TCP/IP stack;
socket translation for guest-to-host connectivity is a different topology.
Either topology still needs trusted attachment identity, mandatory grant
filtering, authenticated cross-host transport, and explicit revocation.

## Candidate software already under consideration

These candidates cover different portions of the path. None is presumed to
provide attachment, trusted identity, grants, name resolution, lifecycle, and
cross-host transport by itself. The linked primary documentation is a starting
point for research; integration and privilege claims need verification in the
intended deployment.

| Candidate and primary documentation | Potential role | Privileges and integration questions |
| --- | --- | --- |
| [passt](https://passt.top/passt/about/) and [manual](https://passt.top/builds/latest/web/passt.1.html) | VM attachment and user-mode TCP/UDP forwarding; DHCP/DNS support | Already unprivileged in the present libkrun provider. Determine whether a supported integration can enforce destination grants before host sockets are opened, preserve source attachment identity, and eliminate alternate egress to protected destinations. Existing passt forwarding alone does not supply the grant model. |
| [pasta](https://passt.top/builds/latest/web/passt.1.html) | Network namespace attachment and forwarding | Related software for namespace connectivity, not a drop-in VM grant enforcer. Identify its namespace/user-namespace requirements and whether it offers a useful boundary for this VM provider. |
| [Linux nftables/Netfilter](https://wiki.nftables.org/wiki-nftables/index.php/Netfilter_hooks) | Kernel packet policy enforcement | Rule administration requires appropriate privileges, normally `CAP_NET_ADMIN` in the relevant network namespace. Host-wide enforcement would change the current host contract. Verify hooks, attachment identity, forwarding/local paths, IPv4/IPv6 coverage, and established-flow revocation. |
| [WireGuard](https://www.wireguard.com/) | Authenticated encrypted cross-host tunnel | A tunnel does not authorize individual workload endpoints. Kernel interface setup normally needs network administration privileges. A userland implementation may still need TUN/device access, interface setup, routes, or privileges unless integrated through a different supported attachment. Record the actual deployment requirements rather than treating userland as automatically rootless. |
| [Caddy reverse proxy](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy) | HTTP reverse proxy | Relevant to existing HTTP service routing. Ordinary PostgreSQL/TCP and UDP forwarding need a different transport facility and trusted source attribution. Caddy HTTP routing alone does not implement private VM network grants. |
| [Optional Caddy layer4 module](https://caddyserver.com/docs/modules/layer4) | TCP/UDP proxy candidate | A separate optional module with its own integration and distribution requirements. Verify supported transport behavior, policy update hooks, per-attachment identity, UDP session handling, and revocation rather than assuming the HTTP module covers them. |
| [gVisor netstack](https://gvisor.dev/docs/architecture_guide/networking/) | User-mode protocol stack and possible attachment/enforcement building block | Evaluate embedding or process integration, guest Ethernet/IP input, host socket output, resolver integration, and insertion points for grant decisions. The network stack does not itself define Heph grants or solve lifecycle and transport. |
| [smoltcp](https://github.com/smoltcp-rs/smoltcp) | Rust protocol stack building block | Evaluate protocol completeness for ordinary clients, IPv4/IPv6, UDP/TCP behavior, performance, and host integration. Its privilege needs depend on the attachment. Using it would leave considerable attachment, forwarding, policy, and lifecycle work to Heph. |
| [Cilium system requirements](https://docs.cilium.io/en/stable/operations/system_requirements/) and [policy introduction](https://docs.cilium.io/en/stable/security/policy/intro/) | Integrated Linux datapath, identity and network policy candidate | Verify privileges, kernel and deployment requirements, VM integration, operational footprint, and whether its policy/lifecycle model can implement the portable contract. Do not assume its standard workload integration matches libkrun's current passt attachment. |

Custom eBPF cgroup socket or packet enforcement is an implementation approach,
not another library. It merits comparison only with an explicit attachment and
privilege model: socket hooks see host socket ownership, which may be the shared
forwarder rather than a guest workload, and packet hooks still need trustworthy
attachment identity. Kernel privilege requirements and enforcement coverage
must be demonstrated.

### Additional candidates

- [x] Screen additional software, including userspace virtual switches, using
  primary documentation after the initial draft exists, recording roles and
  privilege/integration gaps.
  Evidence: primary documentation and source links below reviewed on 2026-09-30.
  This screening establishes possible building blocks, not a validated Heph
  topology, maintenance guarantee, or selected backend.

#### Userspace virtual switches

| Candidate | Documented role | Privileges and remaining integration |
| --- | --- | --- |
| [Open vSwitch userspace/netdev](https://docs.openvswitch.org/en/latest/intro/install/userspace/) | Userspace Ethernet datapath with ingress-port and TCP/UDP field matching, drop actions, and connection tracking. A candidate for mandatory filtering at trusted VM switch ports. | The standard Linux instructions require `/dev/net/tun` and create a local/internal TAP; non-DPDK mode is described as experimental. A completely unprivileged attachment remains unverified. Physical-NIC traffic can also reach the host stack, requiring bypass analysis. |
| [OVN architecture](https://www.ovn.org/support/dist-docs/ovn-architecture.7.html) and [controller](https://www.ovn.org/support/dist-docs/ovn-controller.8.html) | Controller layer providing logical switches/routers, ACLs, DHCP, and DNS over OVS; `ovn-bridge-datapath-type` supports netdev. | OVN and OVS are services and a datapath, rather than a small embedded library. Northbound/Southbound databases and controllers add integration cost. Heph still owns workload identity and translation of bounded endpoint grants into policy. |
| [VDE / vde_switch](https://github.com/virtualsquare/vde-2) | Socket-only userspace Ethernet switch; upstream explicitly demonstrates operation without root. Socket permissions, user/group admission, VLANs, and port closure provide attachment controls. | Host TAP and routing are separate privileged paths. Those attachment controls do not establish endpoint TCP/UDP grants, reply state, or lease/revocation semantics. Heph needs trusted ingress-port mapping and mandatory grant filtering; guest IP/MAC claims remain untrusted. |
| [GNS3 uBridge](https://github.com/GNS3/ubridge) | Userspace bridging across UDP, Unix datagram, TAP, pcap, and raw links, with runtime packet drop filters. | Standard installation grants `CAP_NET_ADMIN` and `CAP_NET_RAW`; raw/TAP modes require privileges. Ordinary UDP ports and accessible Unix sockets suggest a rootless socket-link path from source inspection, not a verified Heph deployment. Socket reachability alone does not authenticate a VM sender. |

OVS's [`--user` option](https://www.openvswitch.org/support/dist-docs/ovs-vswitchd.8.html)
can retain network administration/raw capabilities. The
[DPDK deployment](https://docs.openvswitch.org/en/latest/intro/install/dpdk/)
has separate provisioning requirements. Its
[vhost-user attachment](https://docs.openvswitch.org/en/latest/topics/dpdk/vhost-user/)
uses a negotiated Unix protocol, not libkrun's present passt frame stream.
The [field](https://www.openvswitch.org/support/dist-docs/ovs-fields.7.html) and
[action](https://www.openvswitch.org/support/dist-docs/ovs-actions.7.html) references
document policy primitives; safe grant updates and connection-state flushing
still require a design and acceptance evidence.

The [OVN northbound schema](https://www.ovn.org/support/dist-docs/ovn-nb.5.html)
makes security defaults consequential: empty `port_security` permits addresses,
unmatched ACL traffic is allowed by default, and `persist-established` affects
established-flow handling. A Heph integration needs explicit default denial,
anti-spoofing, and validated revocation of existing flows.

[libvdeplug source](https://github.com/virtualsquare/vde-2/blob/master/src/lib/libvdeplug.c)
uses a Unix stream control channel and Unix datagrams for frame data.
uBridge's [Unix](https://github.com/GNS3/ubridge/blob/master/src/nio_unix.c) and
[UDP](https://github.com/GNS3/ubridge/blob/master/src/nio_udp.c) links likewise need
an adapter for the current VM frame stream. uBridge's TCP control channel is
management, not VM frame transport; same-UID Unix management credentials do not
identify an individual VM.

uBridge's [packet filters](https://github.com/GNS3/ubridge/blob/master/doc/packet_filter.md)
are userspace BPF drop filters available for all backing link types. Each bridge
shares one filter chain between both directions. This runs separately from
privileged kernel eBPF. No stateful grant/UDP
reply engine was established. Policy compilation, authenticated attachments and
tunnels, default denial, filter updates, and revocation remain Heph integration
questions.

#### VM and user-mode stacks

| Candidate | Documented role and attachment | Privileges and remaining integration |
| --- | --- | --- |
| [libslirp API](https://qemu.googlesource.com/libslirp/+/refs/heads/master/src/libslirp.h) and [QEMU user-mode networking](https://www.qemu.org/docs/master/system/devices/net.html) | C library accepting Ethernet frames through `slirp_input` and returning frames through a `send_packet` callback; supports IPv4/IPv6, TCP/UDP, DHCP, and DNS. | Directly feeding VM frames can avoid host TAP, route, and firewall setup. Heph still needs framing, event-loop integration, attachment identity, grants, DNS lifecycle, and any tunnel. Restricted mode and host-loopback controls are coarse controls; the documented TCP guest-forward callback does not establish equivalent UDP grant mediation. |

[Rust libslirp bindings](https://docs.rs/crate/libslirp/latest) and their
[upstream repository](https://gitlab.freedesktop.org/slirp/libslirp-rs) may reduce
FFI work. Binding maintenance and API coverage remain unverified.

#### Embedded overlays

| Candidate | Documented role and attachment | Privileges and remaining integration |
| --- | --- | --- |
| [Tailscale tsnet](https://tailscale.com/docs/features/tsnet), [Go API](https://pkg.go.dev/tailscale.com/tsnet), and [server source](https://github.com/tailscale/tailscale/blob/main/tsnet/tsnet.go) | Embedded overlay node exposing TCP dial/listen and UDP packet listeners, including `udp4`/`udp6`. Its documented rootless path uses a fake TUN and gVisor stack. The server also exposes a `tun.Device` packet-I/O attachment. | A custom in-memory device is a possible VM adapter, but no shipped libkrun Unix/Ethernet adapter was verified. Physical OS TUN attachment changes the privilege requirements. The SDK alone needs application changes or a Heph proxy. Enrollment and policy distribution involve a coordination server; `ControlURL` permits a custom server. Workload generations, service grants, shared attachment attribution, and flow revocation still need a Heph mapping. |
| [ZeroTier libzt](https://github.com/zerotier/libzt), [socket API](https://github.com/zerotier/libzt/blob/main/include/ZeroTierSockets.h), and [Sockets documentation](https://docs.zerotier.com/sockets/) | C/C++ embedded sockets SDK with an internal lwIP stack for TCP/UDP; Rust bindings are listed. This is an application socket API, with no verified transparent VM NIC adapter. | The [internal VirtualTap source](https://github.com/zerotier/libzt/blob/main/src/VirtualTap.cpp) suggests an embedded path without an OS TAP; rootless operation in the proposed topology is an inference requiring validation. A [membership controller](https://docs.zerotier.com/controller/) and [stateless network rules](https://docs.zerotier.com/rules/) leave reply attribution and revocation semantics to establish. The repository's [BSL licensing](https://github.com/zerotier/libzt#licensing), chosen release, and Rust integration need review. |

#### Tunnel and DNS libraries

| Candidate | Documented role and input/output | Privileges and remaining integration |
| --- | --- | --- |
| [BoringTun](https://github.com/cloudflare/boringtun) and [Rust tunnel API](https://github.com/cloudflare/boringtun/blob/master/boringtun/src/noise/mod.rs) | Rust WireGuard cryptography, handshake, and timers with IP-packet input/output. The library does not itself supply the network stack or TUN setup. | Its Linux CLI uses TUN and requires network administration authority. Library integration could use another packet attachment; Heph must provide that attachment, outer UDP transport, peer/key/address routing, grants, and lifecycle. The README warns of restructuring on the main branch, making release selection necessary. |
| [wireguard-go executable](https://git.zx2c4.com/wireguard-go/tree/main.go), [embedded stack](https://git.zx2c4.com/wireguard-go/tree/tun/netstack/tun.go), and [client](https://git.zx2c4.com/wireguard-go/tree/tun/netstack/examples/http_client.go)/[server examples](https://git.zx2c4.com/wireguard-go/tree/tun/netstack/examples/http_server.go) | Go WireGuard implementation whose default executable uses OS TUN. Official examples instead embed gVisor netstack without host TUN, with IPv4/IPv6 TCP/UDP support. | [OS TUN setup](https://docs.kernel.org/networking/tuntap.html) needs device access and network administration permissions. The embedded examples establish a path without host TUN, not a verified rootless Heph VM topology. Go helper/IPC integration, VM packet attachment, and grants remain. WireGuard `AllowedIPs` constrains peer addresses rather than endpoint/protocol/port authority. |
| [Quinn](https://quinn-rs.github.io/quinn/quinn.html) and [data-transfer documentation](https://quinn-rs.github.io/quinn/quinn/data-transfer.html) | Rust QUIC transport with reliable streams and unreliable datagrams; `quinn-proto` separates protocol state from I/O. Potential cross-host tunnel or flow-forwarding building block. | Ordinary UDP sockets can run unprivileged subject to bind policy; elevated socket-buffer configuration is optional. Heph would supply framing, packet/flow adaptation, peer identity, admission, and revocation. QUIC transport does not create a transparent TCP/UDP guest network, and TLS peer authentication does not decide endpoint grants. |
| [Hickory DNS](https://hickory-dns.org/) and [configuration documentation](https://hickory-dns.org/config/) | Rust authoritative-server and resolver libraries, serving DNS over UDP/TCP with optional encrypted transports. Potential private service-name resolver. | High-port sockets need no special network privilege; host port 53 requires bind authority or delegated setup under host policy. A virtual NIC can intercept guest DNS port 53 without binding host port 53. Heph owns naming, scoped answers, guest delivery, and cache lifecycle; DNS restrictions do not protect direct IP connections. |

#### eBPF authoring libraries

| Candidate | Documented role | Privileges and remaining integration |
| --- | --- | --- |
| [Aya](https://github.com/aya-rs/aya) and [cgroup socket-address hooks](https://github.com/aya-rs/aya/blob/main/aya/src/programs/cgroup_sock_addr.rs) | Rust eBPF authoring, loading, and map APIs without libbpf; can express cgroup socket-address programs. | Heph must implement policy programs, attachment identity, coverage, revocation, installation, and cleanup. Existing per-VM worker/passt cgroups could provide trusted source attribution for some host-socket hooks, but that enforcement topology has not been proven. |
| [libbpf-rs](https://github.com/libbpf/libbpf-rs) | Rust bindings to C libbpf, with `libbpf-cargo` build and skeleton support. Another authoring/loading option for a custom kernel enforcer. | It leaves the same policy and lifecycle responsibilities with Heph; the bindings do not grant kernel privileges. |

Both libraries remain subject to
[kernel BPF permission checks](https://github.com/torvalds/linux/blob/master/kernel/bpf/syscall.c),
kernel configuration, and hook availability. Network XDP, TC, and cgroup
socket-address programs require the applicable network administration and BPF
authority. Choosing Rust APIs does not make these enforcement paths rootless.

## Design and research checklist

- [ ] **Define endpoint and grant semantics.**
  - [ ] Specify endpoint registration, ownership, names, protocol/port bindings,
    discovery scope, and collisions across projects and instances.
  - [ ] Specify B's subject binding: exact runtime session, generation, intended
    project/instance scope, and whether any stable service identity is allowed.
  - [ ] Define connect-grant creation, bounded delegation, expiry, renewal,
    revocation, deletion, audit attribution, and cross-project sharing rules.
  - [ ] Define the treatment of reply traffic and separately initiated reverse
    connections; response traffic must not create an unrelated connect grant.
  - [ ] Document how network grants compose with current authorization and
    runtime authority without introducing PostgreSQL credentials into the
    network capability.

- [ ] **Prove the enforcement boundary for candidate topologies.**
  - [ ] Compare private Ethernet forwarding using guest TCP/UDP stacks with
    socket-translated guest-to-host forwarding; identify mandatory grant filters
    and any host-stack dependencies in each topology.
  - [ ] Verify adapters between the current VM Unix frame stream, switch Unix
    datagrams, and negotiated vhost-user attachments, including trusted source
    binding and fail-closed behavior.
  - [ ] Draw rootless user-mode and provisioned privileged alternatives with
    every guest, host, namespace, forwarder, resolver, and cross-host path.
  - [ ] Demonstrate how each attachment authenticates the exact VM/session and
    rejects guest-controlled IP/MAC/source-address identity claims.
  - [ ] Account for private IP scans, direct port access, loopback forwards,
    host-reachable sockets, alternate NICs, passt egress, public addresses that
    route back to private services, and guest-controlled routes.
  - [ ] Establish deny-by-default enforcement for TCP and UDP, IPv4 and IPv6,
    including unsupported families/protocols being rejected rather than
    silently bypassing policy.
  - [ ] Resolve where UDP datagrams are authorized and how reply attribution,
    idle state, spoofing, fragmentation, and QUIC connection migration interact
    with the grant boundary.
  - [ ] Record required host privileges, namespaces, devices, capabilities,
    kernel facilities, and one-time provisioning for each viable backend.
  - [ ] Show that the rootless alternative has no unmediated passt or other
    attachment path to protected destinations; explain any required change to
    the existing provider contract.

- [ ] **Specify lifecycle and failure behavior.**
  - [ ] Choose and state a measurable revocation bound for new connections,
    established TCP streams, and active UDP/QUIC traffic. Decide whether flows
    close immediately or use a bounded grace period, and identify its purpose.
  - [ ] Define policy-cache lifetime, control-plane outage handling, stale
    decisions, failed policy updates, and enforcement-process restart behavior.
    A failed update must not leave an undocumented permissive fallback.
  - [ ] Define cleanup and identity reuse after VM shutdown, replacement,
    endpoint deletion, session expiry, or host failure, including protection
    against an old grant reaching a new workload at a reused address.
  - [ ] Define ordering of endpoint publication, grant installation, readiness,
    revocation acknowledgement, and teardown so partial failures are explicit.
  - [ ] Specify authenticated cross-host peer transport, workload attribution,
    key rotation, replay protections, and host compromise boundaries separately
    from endpoint authorization.
  - [ ] Define private DNS answers, TTLs, negative caching, endpoint movement,
    and resolver outages while retaining enforcement on direct IP access.

- [ ] **Evaluate software and select a design.**
  - [ ] Verify the primary documentation and deployment requirements for every
    listed candidate; distinguish existing features from proposed integration.
  - [ ] Compare feasible combinations for attachment, enforcement, tunnel,
    resolver, and protocol stack, including maintenance and operational cost.
  - [ ] Identify whether a small adapter around maintained software suffices
    before proposing a bespoke full protocol stack or datapath.
  - [ ] Produce a decision record covering the selected backend, rejected
    alternatives, privilege model, limitations, and unresolved dependencies.
  - [ ] Specify portable core interfaces and standard provider responsibilities
    without leaking nftables, Caddy, WireGuard, or stack-specific objects into
    the domain contract.
  - [ ] Turn an accepted design into a bounded implementation plan before
    changing production networking code.

## Reference acceptance scenarios

- [ ] A exposes TCP `postgresql-XXX:5432`; an ordinary PostgreSQL client in B
  reaches it under an exact grant, and database authentication still applies.
- [ ] An ungranted VM C cannot connect through the name, a cached DNS answer,
  the direct IPv4/IPv6 address, guessed ports, or a scanned host forward.
- [ ] B cannot reach another endpoint or protocol/port on A, and its delegated
  grant cannot be broadened or transferred outside its authorized subject scope.
- [ ] A malicious guest cannot impersonate B by changing source IP/MAC,
  transmitting forged packets, or claiming an authorized runtime identity.
- [ ] An authorized UDP application and a QUIC application exchange traffic;
  ungranted datagrams and separately initiated reverse traffic are denied.
- [ ] Revocation and expiry affect new TCP connections, existing TCP streams,
  and active UDP/QUIC traffic within the documented bounds, on one and multiple
  hosts.
- [ ] VM replacement and address/name reuse cannot make an old grant authorize
  a new runtime, and teardown removes obsolete attachments and flow state.
- [ ] Policy service outage, policy update failure, resolver outage, and
  enforcement restart follow the documented failure behavior with bounded stale
  authority and no bypass route.
- [ ] A selected rootless design is exercised with the actual unprivileged
  runtime account and proves that every route to protected endpoints, including
  existing passt egress, crosses grant enforcement. Any selected privileged
  design is instead exercised with its explicit provisioning contract.
- [ ] Cross-host tests reject unauthenticated peers, incorrect workload
  attribution, and replayed or expired authority.

## Eventual implementation verification

These gates apply to a subsequent implementation; writing this draft does not
complete them.

- [ ] Add focused tests and real VM acceptance coverage for the selected design
  and the scenarios above, including rootless/privileged deployment as selected.
- [ ] Document provider configuration, provisioning, privileges, revocation
  bounds, outages, DNS behavior, and operator debugging procedures.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --all-features`.
- [ ] Run `cargo test --workspace --all-features`.
- [ ] Run `cargo doc --workspace --all-features --no-deps`.
- [ ] Run the repository handoff gate `cargo dev quality`.
- [ ] Record command results, acceptance evidence, remaining limitations, and
  deliberate follow-up tasks before moving an implementation task to done.

## Completion evidence

Additional-candidate primary-documentation screening, expanded to userspace
virtual switches and OVN's controller role, was completed on 2026-09-30. The
linked findings above distinguish documented software interfaces from proposed
integration and source-based inference. No prototype, deployment validation,
networking implementation, or implementation gate has been run.
Backend selection, topology and privilege validation, lifecycle guarantees, and
acceptance evidence remain unchecked work above.
