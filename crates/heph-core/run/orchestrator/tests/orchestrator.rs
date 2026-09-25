//! Hardware-independent orchestration and cleanup-ordering coverage.

#[path = "orchestrator/completion.rs"]
mod completion;
#[path = "orchestrator/launch.rs"]
mod launch;
#[path = "orchestrator/recovery.rs"]
mod recovery;
#[path = "orchestrator/resource.rs"]
mod resource;
#[path = "orchestrator/support/mod.rs"]
mod support;
