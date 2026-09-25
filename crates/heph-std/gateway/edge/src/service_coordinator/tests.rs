use super::*;
use crate::{
    GatewayEdgeError, GatewayLimits, GatewayRequest, GatewayScheme, GatewayServiceFailure,
    GatewayServiceFailureStoreError, GatewayServiceLaunch, GatewayServiceLeasePolicy,
    ServiceHttpPolicy, ServiceInstancePolicy, TrustedRequestMetadata,
};
use async_trait::async_trait;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use std::{
    collections::BTreeMap,
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Notify, broadcast},
    time::timeout,
};
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, PrivateHttpServiceSpec, RootFilesystem, StopMode, VmError, VmEvent,
    VmExit, VmId, VmResources, VmSpec,
};

#[path = "service_coordinator_tests/drain_fixture.rs"]
mod drain_fixture;
#[path = "service_coordinator_tests/fixture_values.rs"]
mod fixture_values;
#[path = "service_coordinator_tests/ownership_mocks.rs"]
mod ownership_mocks;
#[path = "service_coordinator_tests/provider_mocks.rs"]
mod provider_mocks;
#[path = "service_coordinator_tests/target_mocks.rs"]
mod target_mocks;
use drain_fixture::*;
use fixture_values::*;
use ownership_mocks::*;
use provider_mocks::*;
use target_mocks::*;

#[path = "service_coordinator_tests/drain_behavior.rs"]
mod drain_behavior;
#[path = "service_coordinator_tests/service_log_lifecycle.rs"]
mod service_log_lifecycle;
#[path = "service_coordinator_tests/startup_lifecycle.rs"]
mod startup_lifecycle;
use startup_lifecycle::respond_status_after_gate;

#[path = "service_coordinator_tests/cleanup_lifecycle.rs"]
mod cleanup_lifecycle;
#[path = "service_coordinator_tests/health_behavior.rs"]
mod health_behavior;
#[path = "service_coordinator_tests/health_cancellation.rs"]
mod health_cancellation;
#[path = "service_coordinator_tests/health_lease.rs"]
mod health_lease;
#[path = "service_coordinator_tests/promotion_lifecycle.rs"]
mod promotion_lifecycle;
#[path = "service_coordinator_tests/promotion_provision.rs"]
mod promotion_provision;
#[path = "service_coordinator_tests/restore_lifecycle.rs"]
mod restore_lifecycle;

#[path = "service_coordinator_tests/service_log_writer.rs"]
mod service_log_writer;
use service_log_writer::LifecycleLogStore;
