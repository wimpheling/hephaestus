//! Authenticated retirement checks for the cooking acceptance path.
//!
//! Resource changes use generated RPCs or the existing authenticated release
//! service command. SQL is limited to read-only effect and durable-history
//! inspection.

#[path = "retirement/assertions.rs"]
mod assertions;
#[path = "retirement/attachment.rs"]
mod attachment;
#[path = "retirement/exercise.rs"]
mod exercise;
#[path = "retirement/gateway.rs"]
mod gateway;
#[path = "retirement/release.rs"]
mod release;
#[path = "retirement/rpc.rs"]
mod rpc;
#[path = "retirement/types.rs"]
mod types;
#[path = "retirement/wait.rs"]
mod wait;

pub(crate) use assertions::{assert_final_provenance, effect_counts};
pub(crate) use attachment::remove_attachment_and_assert;
pub use exercise::exercise;
pub(crate) use gateway::retire_gateway;
pub(crate) use release::retire_release;
pub(crate) use rpc::{
    gateway_client, instance_client, mutation_context, opaque, release_client, run_client,
};
pub use types::RetirementContext;
pub(crate) use types::{ProvenanceBaseline, RetryObservation};
pub(crate) use wait::{wait_for_control_terminal, wait_for_failed_retry};
