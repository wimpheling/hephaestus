//! Real `PostgreSQL` coverage for durable gateway service ownership.

use gateway_domain::{
    GatewayServiceFailure, GatewayServiceFailureCode, GatewayServiceFailureStore,
    GatewayServiceFailureStoreError, GatewayServiceInstanceState, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_BATCH,
};
use gateway_postgres::{PostgresGatewayServiceFailureStore, PostgresGatewayServiceOwnership};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};
use uuid::Uuid;

#[path = "gateway_service_ownership/support/mod.rs"]
mod support;
use support::*;

#[path = "gateway_service_ownership/claim_fence.rs"]
mod claim_fence;
#[path = "gateway_service_ownership/cleanup.rs"]
mod cleanup;
#[path = "gateway_service_ownership/drain.rs"]
mod drain;
#[path = "gateway_service_ownership/failure_fence.rs"]
mod failure_fence;
#[path = "gateway_service_ownership/failure_reset_lock.rs"]
mod failure_reset_lock;
#[path = "gateway_service_ownership/failure_retry.rs"]
mod failure_retry;
#[path = "gateway_service_ownership/failure_revision.rs"]
mod failure_revision;
#[path = "gateway_service_ownership/promotion.rs"]
mod promotion;
#[path = "gateway_service_ownership/promotion_instance_lock.rs"]
mod promotion_instance_lock;
#[path = "gateway_service_ownership/promotion_release_lock.rs"]
mod promotion_release_lock;
#[path = "gateway_service_ownership/renewal_lock.rs"]
mod renewal_lock;
#[path = "gateway_service_ownership/revision_host.rs"]
mod revision_host;
#[path = "gateway_service_ownership/superseded.rs"]
mod superseded;
#[path = "gateway_service_ownership/trigger.rs"]
mod trigger;

#[path = "gateway_service_ownership/claim_resolution.rs"]
mod claim_resolution;
#[path = "gateway_service_ownership/coordinator/mod.rs"]
mod coordinator;
#[path = "gateway_service_ownership/expired_takeover/mod.rs"]
mod expired_takeover;
