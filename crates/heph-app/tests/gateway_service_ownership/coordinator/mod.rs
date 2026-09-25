//! Real `PostgreSQL` coordinator integration using a deterministic fake VM.

use super::{active_pointer, seed_fixture, seed_service_revision, test_pool};
use async_trait::async_trait;
use gateway_domain::{
    GatewayLimits, GatewayRequest, GatewayScheme, GatewayServiceFailureCode,
    GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceLaunch,
    GatewayServiceLaunchRequest, GatewayServiceLaunchResolver, GatewayServiceOwner,
    GatewayServiceOwnership, TrustedRequestMetadata,
};
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayServiceCoordinator, GatewayServiceCoordinatorStatus, GatewayServiceLeasePolicy,
    GatewayServiceRegistry, GatewayServiceStartupIntent, GatewayServiceSupervisorPolicy,
    ServiceHttpPolicy, ServiceInstancePolicy,
};
use gateway_postgres::{
    PostgresGatewayServiceFailureStore, PostgresGatewayServiceOwnership,
    PostgresGatewayServiceTargets,
};
use http::{HeaderMap, Method, StatusCode};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, duplex},
    sync::{Notify, broadcast},
    time::{Instant, timeout},
};
use uuid::Uuid;
use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpServiceSpec,
    RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmResources,
    VmSpec,
};

fn coordinator_policy(
    instance: ServiceInstancePolicy,
    lease: GatewayServiceLeasePolicy,
) -> GatewayServiceSupervisorPolicy {
    GatewayServiceSupervisorPolicy {
        instance,
        lease,
        ..GatewayServiceSupervisorPolicy::default()
    }
}

#[path = "fakes.rs"]
mod fakes;
#[path = "scenario_b.rs"]
mod scenario_b;
#[path = "scenarios_a.rs"]
mod scenarios_a;

pub use fakes::*;
