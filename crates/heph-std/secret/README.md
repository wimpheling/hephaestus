# Purpose

This directory contains the standard secret adapters that connect core secret
authority to encrypted storage, PostgreSQL lifecycle state, ephemeral guest
mounts, and the host broker. They turn an authorized lease into either a
short-lived read-only mount or a bounded broker operation.

# Responsibilities

The local key provider loads versioned wrapping keys, PostgreSQL owns secret
commands and runtime provenance, the runtime provider creates and reconciles
ephemeral files, and the broker validates host-mediated HTTPS requests. Each
adapter keeps the plaintext boundary in the narrow operation that needs it.

# When

Compose the key provider and encrypted store first, then give them to the
PostgreSQL secret services. Add the runtime provider for raw delivery and the
broker for brokered delivery before installing the run secret manager.
