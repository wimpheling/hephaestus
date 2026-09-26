//! Authenticated retirement proof for the cooking gateway publication grant.

use super::GatewayGoldenFixture;
use std::error::Error;

type AuthorityError = Box<dyn Error + Send + Sync>;

#[path = "authority/retirement.rs"]
mod retirement;
#[path = "authority/rotation.rs"]
mod rotation;
#[path = "authority/rpc.rs"]
mod rpc;
#[cfg(test)]
#[path = "authority/tests.rs"]
mod tests;

pub use rotation::{assert_inbound_lease_history, configure_rotated_inbound_gateway};
pub(crate) use rpc::{
    gateway_client, gateway_client_with_token, gateway_id, mutation_context, opaque, outsider_token,
};
