# Purpose

`registry-reconciler` turns lossy Zot notifications into authoritative Forge
publication state. It re-reads Zot by the exact immutable digest, compares the
observation with the durable publication intent, and emits a typed action for
an authorized executor.

# Responsibilities

`RegistryReconciler` claims one notification or scans publication intents,
inspects the registry through `ZotRegistry`, and reduces the result into
approval, retry, missing, or inconsistency actions. `NotificationInbox` and
`PublicationIntents` make claims and completion durable, while
`ReconciliationActionExecutor` applies lifecycle mutations after the reducer
has made its decision.

The callback itself never grants approval. Exact digest comparison, namespace
ownership, publication intent, and verification evidence determine whether a
publication can become approved or must be retried or marked missing.

# When

Run `RegistryReconciler::process_next` for callback-driven work and
`reconcile_all` for recovery or periodic drift checks. Use the `*_and_apply`
variants when the caller also owns the authorized action executor.
