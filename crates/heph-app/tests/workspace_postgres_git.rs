//! Opt-in exact-commit and controlled-result integration coverage.

use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use forge_storage::GitStorage;
use heph_forge::{CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate};
use heph_run::StartRun;
use heph_runtime::{
    ResultRepository, RunWorkspaceManager, RuntimeGitWorkspaceRequest, WorkspaceMetadata,
    WorkspaceMetadataRepository,
};
use run_orchestrator::{RunRepository, RunRuntimeCatalog};
use run_postgres::PgRunRepository;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::process::Command;
use uuid::Uuid;
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager, WorkspaceLimits};
use workspace_postgres::PgWorkspaceMetadataRepository;

#[path = "workspace_postgres_git/support/mod.rs"]
mod support;
use support::*;

#[path = "workspace_postgres_git/controlled_result.rs"]
mod controlled_result;
#[path = "workspace_postgres_git/resolver.rs"]
mod resolver;
