//! Real `PostgreSQL` proof for daemon-owned service invocation recovery.

use super::{
    gateway_reconciliation_loop, gateway_reconciliation_loop_with_boot,
    gateway_reconciliation_loop_with_context,
};
use async_trait::async_trait;
use bytes::Bytes;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayLimits,
    GatewayProvider, GatewayProviderResponse, GatewayRequest, GatewayResponse,
    GatewayServiceBootRecovery, GatewayServiceBootRecoveryContext,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanupDriverPolicy,
    GatewayServiceInstanceLease, GatewayServiceInstancePage, GatewayServiceInstancePageResult,
    GatewayServiceLaunch, GatewayServiceLaunchRequest, GatewayServiceLaunchResolver,
    GatewayServiceLogStore, GatewayServiceLogWriterConfig, GatewayServiceOwnedTarget,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceOwnershipError,
    GatewayServiceRegistry, GatewayServiceSupervisor, GatewayServiceSupervisorContext,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    GatewayServiceTargetStore, ServiceLogWriterPolicy,
};
use gateway_postgres::{
    PostgresGatewayEdgeAuthority, PostgresGatewayServiceFailureStore,
    PostgresGatewayServiceLogStore, PostgresGatewayServiceOwnership, PostgresGatewayServiceTargets,
};
use http::{HeaderMap, StatusCode};
use serial_test::serial;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    collections::BTreeMap,
    env, fmt,
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration as StdDuration,
};
use time::{Duration, OffsetDateTime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpRequest,
    PrivateHttpResponse, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError, VmEvent, VmId,
    VmInstance, VmProvider, VmResources, VmSpec,
};

/// Provider and fixture test doubles.
#[path = "gateway_recovery_tests/provider_support.rs"]
mod provider_support;
pub use provider_support::*;

/// Ownership and claim resolution test doubles.
#[path = "gateway_recovery_tests/ownership_support.rs"]
mod ownership_support;
pub use ownership_support::*;

/// Target store test doubles.
#[path = "gateway_recovery_tests/target_support.rs"]
mod target_support;
pub use target_support::*;

/// Launch and gateway provider test doubles.
#[path = "gateway_recovery_tests/resolver_support.rs"]
mod resolver_support;

/// Automatic startup loop fixture construction.
#[path = "gateway_recovery_tests/automatic_start.rs"]
mod automatic_start;
use automatic_start::{
    spawn_automatic_start_with_worker, spawn_automatic_start_with_worker_config,
};

/// Shared startup and supervisor database assertions.
#[path = "gateway_recovery_tests/recovery_support.rs"]
mod recovery_support;
use recovery_support::{
    cleanup_startup_fixture, clear_service_log_fixture, make_authority, test_supervisor_context,
    wait_for_ready, wait_for_ready_and_active,
};

/// Isolated database and worker-pool fixtures.
#[path = "gateway_recovery_tests/database.rs"]
mod database;
use database::{
    IsolatedStartupDatabase, drop_isolated_startup_database, isolated_startup_database, test_pool,
    worker_pool, worker_pool_for_options_named,
};

/// Gateway and service seed fixtures.
#[path = "gateway_recovery_tests/fixtures.rs"]
mod fixtures;
use fixtures::{seed_application_log_fixture, seed_fixture};

/// Candidate release and revision fixture.
#[path = "gateway_recovery_tests/candidate.rs"]
mod candidate;
use candidate::seed_service_candidate;

/// Durable invocation, session, and secret lease fixture inserts.
#[path = "gateway_recovery_tests/persistence.rs"]
mod persistence;
use persistence::{insert_host_session, insert_invocation, insert_lease};
/// Production service log scenarios.
#[path = "gateway_recovery_tests/service_log.rs"]
mod service_log;

/// Recovery reconciliation and cancellation scenarios.
#[path = "gateway_recovery_tests/recovery_lifecycle.rs"]
mod recovery_lifecycle;

/// Supervisor polling and claim replacement scenarios.
#[path = "gateway_recovery_tests/supervisor_poll.rs"]
mod supervisor_poll;

/// Claim loss and confirmed absence scenarios.
#[path = "gateway_recovery_tests/claim_resolution.rs"]
mod claim_resolution;

/// Healthy service progress under claim contention.
#[path = "gateway_recovery_tests/healthy_progress.rs"]
mod healthy_progress;

/// Desired revision transition scenarios.
#[path = "gateway_recovery_tests/desired_revision.rs"]
mod desired_revision;

/// Retained cleanup retry scenarios.
#[path = "gateway_recovery_tests/retained_cleanup.rs"]
mod retained_cleanup;

/// Fair target refresh scenarios.
#[path = "gateway_recovery_tests/fairness.rs"]
mod fairness;

/// Startup transition scenarios.
#[path = "gateway_recovery_tests/startup_transitions.rs"]
mod startup_transitions;

/// Boot inventory gate scenario.
#[path = "gateway_recovery_tests/boot_gate.rs"]
mod boot_gate;
