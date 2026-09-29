# Purpose

This directory contains the standard authorization adapters that connect
provider-neutral capability and runtime authority contracts to PostgreSQL and
trusted host storage. Together they turn an approved request or workload into a
durable decision, an inspectable audit record, or a short-lived runtime bearer.

# Responsibilities

The PostgreSQL adapters evaluate request permissions, persist capability
evidence, and store immutable runtime snapshots and hash-only sessions. The
local handoff adapter encrypts temporary bootstrap envelopes on the trusted VM
host, including the separate credential namespace used by runtime Git.

# When

Compose these adapters when deploying the authorization boundary. Use the
PostgreSQL providers for request and runtime persistence, then give the runtime
issuers a host handoff implementation before starting guest bootstrap.
