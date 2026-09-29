pub(super) use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub(super) use ::time::{Duration as TimeDuration, OffsetDateTime};
pub(super) use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
pub(super) use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(super) use tokio::sync::Notify;
pub(super) use vm_trait::{
    BoxedPrivateServiceConnection, GuestCommand, NetworkMode, PrivateHttpServiceSpec,
    RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmResources,
};

pub(super) use super::*;
pub(super) use crate::{
    GatewayEdgeError, GatewayServiceFailure, GatewayServiceFailureStoreError,
    GatewayServiceIdentity, GatewayServiceInstancePage, GatewayServiceInstancePageResult,
    GatewayServiceTargetPage, GatewayServiceTargetPageResult, GatewayServiceTargetStore,
};

#[path = "service_supervisor_tests/helpers.rs"]
mod helpers;
#[path = "service_supervisor_tests/mocks.rs"]
mod mocks;
#[path = "service_supervisor_tests/ready_ownership.rs"]
mod ready_ownership;
#[path = "service_supervisor_tests/ready_targets.rs"]
mod ready_targets;
#[path = "service_supervisor_tests/ready_vm.rs"]
mod ready_vm;

use helpers::*;
use mocks::*;
use ready_ownership::*;
use ready_targets::*;
use ready_vm::*;

#[path = "service_supervisor_tests/capacity.rs"]
mod capacity;
#[path = "service_supervisor_tests/claim_resolution.rs"]
mod claim_resolution;
#[path = "service_supervisor_tests/claim_startup.rs"]
mod claim_startup;
#[path = "service_supervisor_tests/cleanup_resolution.rs"]
mod cleanup_resolution;
#[path = "service_supervisor_tests/cleanup_retry.rs"]
mod cleanup_retry;
#[path = "service_supervisor_tests/lifecycle.rs"]
mod lifecycle;
#[path = "service_supervisor_tests/polling.rs"]
mod polling;
