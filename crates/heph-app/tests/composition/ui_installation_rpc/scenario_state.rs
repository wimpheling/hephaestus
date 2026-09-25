use connectrpc::client::SharedHttp2Connection;
use rpc_proto::{
    connect::hephaestus::release::v1::ReleaseServiceClient,
    messages::hephaestus::release::v1::{InstallUiResponse, UiInstallationTarget},
};

pub(crate) type ReleaseClient = ReleaseServiceClient<SharedHttp2Connection>;

pub(crate) struct InstallScenarioState {
    pub(crate) target: UiInstallationTarget,
    pub(crate) installed: InstallUiResponse,
    pub(crate) list_token: String,
    pub(crate) handoff_token: String,
}
