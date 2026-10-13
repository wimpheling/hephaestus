//! Generated-client boundary for the genuine configured build/publication fixture.
//!
//! RPC requests and responses remain within these private helpers. The exported
//! interface contains only domain IDs, strings, paths and existing fixture context.
//! `build_one` projects generated responses before returning publication provenance.
//! Original Cooking helper bodies and the authenticated build flow are reused.

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
            GatewayLifecycle, GatewayServiceLogScope, GatewayServiceLogStream, GetGatewayRequest,
            InstallReleaseGatewaysRequest, ListGatewayServiceLogsRequest,
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

// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/build_flow.rs"]
mod build_flow;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/data.rs"]
mod data;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/filesystem.rs"]
mod filesystem;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/gateway_wait.rs"]
mod gateway_wait;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/instances.rs"]
mod instances;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/publishing.rs"]
mod publishing;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/queue.rs"]
mod queue;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/rpc.rs"]
mod rpc;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/ui_install.rs"]
mod ui_install;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/ui_list.rs"]
mod ui_list;
// Frozen Cooking helpers retain crate visibility; this private reuse context triggers this lint.
#[allow(clippy::redundant_pub_crate)]
#[path = "../../../../../examples/cooking/tests/builds/ui_validate.rs"]
mod ui_validate;

use data::*;
use filesystem::*;
use gateway_wait::*;
use instances::*;
use publishing::*;
use queue::*;
use rpc::*;
use ui_install::*;
use ui_list::*;
use ui_validate::*;

pub(super) use build_flow::build_one;
pub(super) use data::PublishedCookingRepository;
pub(super) use queue::{BuildError, CookingBuildContext, CookingIdentity};
