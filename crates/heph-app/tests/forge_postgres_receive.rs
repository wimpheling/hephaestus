//! Opt-in `PostgreSQL` and `JetStream` receive-processing coverage.

use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{
    CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate, Repository, RuntimeReceiveProvenance,
};
use forge_postgres::PgForgeRepository;
use forge_service::{
    BUILD_REQUESTED_SUBJECT, CreateRepository, ForgeNatsOutboxPublisher, GitStorage,
    INSTANCE_RUN_REQUESTED_SUBJECT, RUN_START_SUBJECT, ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{
    NatsCommandHandler, RunOrchestrator, RunRepository, VmSpecFactory, ensure_jetstream_topology,
};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio::process::Command;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmResources, VmSpec};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;

const STATIC_UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

#[path = "forge_postgres_receive/accepted_ui.rs"]
mod accepted_ui;
#[path = "forge_postgres_receive/app_role.rs"]
mod app_role;
#[path = "forge_postgres_receive/concurrent.rs"]
mod concurrent;
#[path = "forge_postgres_receive/deduplicated.rs"]
mod deduplicated;
#[path = "forge_postgres_receive/invalid_agent.rs"]
mod invalid_agent;
#[path = "forge_postgres_receive/invalid_ref.rs"]
mod invalid_ref;
#[path = "forge_postgres_receive/outbox_retry.rs"]
mod outbox_retry;
#[path = "forge_postgres_receive/pushed_config.rs"]
mod pushed_config;
#[path = "forge_postgres_receive/runtime_receive.rs"]
mod runtime_receive;
#[path = "forge_postgres_receive/same_commit.rs"]
mod same_commit;
#[path = "forge_postgres_receive/support/mod.rs"]
mod support;

use support::*;
