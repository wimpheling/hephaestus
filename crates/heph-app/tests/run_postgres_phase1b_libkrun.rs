//! Opt-in full Phase 1B persistence test requiring KVM and `PostgreSQL`.

use authz_postgres::PostgresMelangeAuthorizer;
use heph_run::{Run, RunKind, RunOutcome, RunState, StartRun};
use heph_runtime::VolumeStore;
use heph_runtime::{
    GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmProvider, VmResources, VmSpec,
};
use release_domain::AgentUpdateId;
use release_postgres::{ReleaseService, UpdateDecision};
use run_orchestrator::{RepositoryError, RunOrchestrator, RunRepository, VmSpecFactory};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;

const ENABLE_FLAG: &str = "HEPHAESTUS_PHASE1B_INTEGRATION";

#[path = "run_postgres_phase1b_libkrun/candidate.rs"]
mod candidate;
#[path = "run_postgres_phase1b_libkrun/db.rs"]
mod db;
#[path = "run_postgres_phase1b_libkrun/fixture.rs"]
mod fixture;
#[path = "run_postgres_phase1b_libkrun/model.rs"]
mod model;
#[path = "run_postgres_phase1b_libkrun/runtime.rs"]
mod runtime;
#[path = "run_postgres_phase1b_libkrun/scenario.rs"]
mod scenario;
#[path = "run_postgres_phase1b_libkrun/seed.rs"]
mod seed;
