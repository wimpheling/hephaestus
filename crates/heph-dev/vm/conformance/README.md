# Purpose

`vm-conformance` is a reusable asynchronous test suite for implementations of
the `vm-trait` provider contract. It gives each provider the same lifecycle
expectations and reports differences before a provider is accepted and wired
into application composition.

# Responsibilities

`ProviderHarness` supplies a provider and the VM specs it supports. The
exported lifecycle suite checks that provisioning, concurrent start, shared
wait results, stop, destroy, and identifier reuse behave consistently; invalid
core specs return typed errors, and caller-owned paths survive provider
cleanup. Optional capability flags add ingress and ready-event checks when a
provider advertises them.

# When

Use this crate from a VM provider's integration tests. Implement
`ProviderHarness`, call `lifecycle_suite(&harness)`, and add
`invalid_core_specs_are_typed` plus optional tests for the capabilities the
provider supports.
