use super::*;
use crate::{
    GatewayEdgeError, GatewayServiceInstanceKey, GatewayServiceInstancePage,
    GatewayServiceInstancePageResult, GatewayServiceLaunch, GatewayServiceLaunchRequest,
    GatewayServiceOwnedTarget, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
};
use ::time::OffsetDateTime;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use uuid::Uuid;
use vm_trait::{VmError, VmId, VmInstance, VmSpec};

#[path = "cleanup_driver_tests/helpers.rs"]
mod helpers;
#[path = "cleanup_driver_tests/mocks.rs"]
mod mocks;
use helpers::*;
use mocks::*;

#[path = "cleanup_driver_tests/cleanup_scenarios.rs"]
mod cleanup_scenarios;
#[path = "cleanup_driver_tests/failure_scenarios.rs"]
mod failure_scenarios;
#[path = "cleanup_driver_tests/renewal_scenarios.rs"]
mod renewal_scenarios;
