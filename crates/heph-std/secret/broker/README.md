# Purpose

`secret-broker` is the trusted host side of the guest-to-host secret broker.
It accepts a bounded framed request over the private socket exposed to the VM,
authenticates it through the secret runtime service, and performs the approved
semantic operation with a host adapter.

# Responsibilities

`BrokerServer` binds an absolute private Unix socket with mode `0600`, rejects
unsafe objects and invalid frames, and keeps malformed connections from
stopping the listener. `ServiceBrokerExecutor` validates the credential and
slot before invoking the runtime authority boundary. The HTTPS registry uses
control-plane-pinned DNS addresses, HTTPS-only connections, certificate and
SNI verification, no redirects, bounded bodies, and header substitution that
rejects credential reflection.

# When

Build a `ServiceBrokerExecutor` with the runtime resolver and a configured
adapter, pass it to `BrokerServer::bind`, and serve until shutdown
cancellation. Use `BrokeredHttpsAdapterRegistry` for immutable outbound rules;
use the loopback adapter only in its explicitly bounded integration-test flow.
