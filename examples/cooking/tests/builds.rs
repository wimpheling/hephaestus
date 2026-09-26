//! Helpers for exercising the cooking build and release path through the app.
//!
//! The golden fixture passes its database, daemon, and identity context here
//! so these operations exercise the production RPC boundaries.

use forge_domain::{GitRef, OrganizationId, ProjectId, RepositoryId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use hephaestus_app::RunningHephaestus;
use identity_domain::{AuthenticatedIdentity, UserId};
use rpc_proto::{
    connect::hephaestus::{
        build::v1::BuildServiceClient, gateway::v1::GatewayServiceClient,
        instance::v1::AgentInstanceServiceClient, release::v1::ReleaseServiceClient,
    },
    messages::hephaestus::{
        build::v1::{BuildState, GetBuildRequest},
        common::v1::{
            Cursor, NetworkPolicy, OpaqueId, PageRequest, ParameterValue, RequestContext,
            RuntimePolicy,
        },
        gateway::v1::{
            ConfigureGatewayRequest, CreateMailboxBindingRequest, GatewayLifecycle,
            GatewaySecretSelection, GatewayServiceLogScope, GatewayServiceLogStream,
            GetGatewayRequest, InstallReleaseGatewaysRequest, ListGatewayServiceLogsRequest,
            ListProjectGatewaysRequest,
        },
        instance::v1::{
            CreateAttachmentRequest, CreateMailboxRequest, ImportAgentRequest, RefSelector,
            TriggerPolicy, ref_selector,
        },
        release::v1::{
            ActivateUiRequest, DisableUiRequest, GetReleaseRequest, InstallUiRequest,
            ListUiInstallationsRequest, PublishReleaseRequest, ReleaseUiPresentation,
            ReleaseUiScope, RemoveUiRequest, SetDraftVersionRequest, UiInstallationContentKind,
            UiInstallationLifecycle, UiInstallationNavigation, UiInstallationTarget,
            release_ui_descriptor, ui_installation_target,
        },
    },
};
use serde_json::Value;
use sqlx::PgPool;
use std::{
    error::Error,
    fs, io,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{process::Command, time::sleep};
use uuid::Uuid;

#[path = "builds/adversarial.rs"]
mod adversarial;
#[path = "builds/blog.rs"]
mod blog;
#[path = "builds/build_flow.rs"]
mod build_flow;
#[path = "builds/data.rs"]
mod data;
#[path = "builds/filesystem.rs"]
mod filesystem;
#[path = "builds/gateway.rs"]
mod gateway;
#[path = "builds/gateway_wait.rs"]
mod gateway_wait;
#[path = "builds/instances.rs"]
mod instances;
#[path = "builds/publishing.rs"]
mod publishing;
#[path = "builds/queue.rs"]
mod queue;
#[path = "builds/rpc.rs"]
mod rpc;
#[path = "builds/ui_install.rs"]
mod ui_install;
#[path = "builds/ui_list.rs"]
mod ui_list;
#[path = "builds/ui_validate.rs"]
mod ui_validate;
#[path = "builds/unit_tests.rs"]
mod unit_tests;

pub(crate) use adversarial::*;
pub(crate) use blog::*;
pub(crate) use build_flow::*;
pub(crate) use data::*;
pub(crate) use filesystem::*;
pub(crate) use gateway::*;
pub(crate) use gateway_wait::*;
pub(crate) use instances::*;
pub(crate) use publishing::*;
pub(crate) use queue::*;
pub(crate) use rpc::*;
pub(crate) use ui_install::*;
pub(crate) use ui_list::*;
pub(crate) use ui_validate::*;
