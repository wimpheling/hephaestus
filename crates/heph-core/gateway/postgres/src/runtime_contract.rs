//! Provider-neutral runtime contract decoding for gateway releases.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct GatewayRuntimeContract {
    #[serde(alias = "executable")]
    pub(crate) command: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) working_directory: String,
    pub(crate) image_reference: String,
    pub(crate) requires_state: bool,
    pub(crate) policy_ceiling: GatewayPolicyCeiling,
}

#[derive(Deserialize)]
pub struct GatewayPolicyCeiling {
    pub(crate) vcpus: u8,
    pub(crate) memory_mib: u32,
    pub(crate) network: GatewayNetwork,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayNetwork {
    Disabled,
    BrokerOnly,
    Egress,
}
