mod common;
mod containment;
mod drain;
mod helpers;
mod http;
mod instance;
mod launch;
mod lifecycle;
mod lifecycle_task;
mod ownership;
mod provider_api;
mod registry;
mod worker;
mod worker_client;
mod worker_process;

pub use provider_api::{LibkrunProvider, ProspectiveOwnedVmBootstrap};
pub use registry::{VmAttemptInventory, VmAttemptPhase, VmAttemptSnapshot};

#[cfg(test)]
mod tests;
