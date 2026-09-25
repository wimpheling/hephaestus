use connectrpc::client::SharedHttp2Connection;
use rpc_proto::{
    connect::hephaestus::release::v1::ReleaseServiceClient,
    messages::hephaestus::release::v1::{InstallUiResponse, UiInstallationTarget},
};

pub(super) type ReleaseClient = ReleaseServiceClient<SharedHttp2Connection>;

pub(super) struct InstallScenarioState {
    pub(super) target: UiInstallationTarget,
    pub(super) installed: InstallUiResponse,
    pub(super) list_token: String,
    pub(super) handoff_token: String,
}
