# Purpose

`secret-service` preserves the service-facing name for the secret application
API. A service depending on it receives the same command, dispatch, runtime,
and broker contracts used to manage secret authority and prepare runs.

# Responsibilities

The crate re-exports the current `secret-application` API, allowing existing
service wiring to use the same command, dispatch, runtime, and broker flows
without changing its dependency name. That keeps callers on one set of typed
errors and the same security-sensitive value boundaries while the service
code moves through the current secret workflow.

# When

Use this crate from a service that still imports the compatibility package
name, then call the same application ports:

```rust
use secret_service::{SecretCommandService, SecretDispatchResolver};
```
