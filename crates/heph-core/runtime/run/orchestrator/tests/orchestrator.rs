//! Hardware-independent orchestration and cleanup-ordering coverage.

#[path = "orchestrator/canonical.rs"]
mod canonical;

#[path = "orchestrator/completion.rs"]
mod completion;
#[path = "orchestrator/invocation.rs"]
mod invocation;
#[path = "orchestrator/launch.rs"]
mod launch;
#[path = "orchestrator/legacy_placement.rs"]
mod legacy_placement;
#[path = "orchestrator/recovery.rs"]
mod recovery;
#[path = "orchestrator/resource.rs"]
mod resource;
#[path = "orchestrator/strict_legacy.rs"]
mod strict_legacy;
#[path = "orchestrator/support/mod.rs"]
mod support;
