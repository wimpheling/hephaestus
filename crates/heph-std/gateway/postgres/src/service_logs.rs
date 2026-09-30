//! Worker-only durable append adapter for opt-in application service logs.

use gateway_domain::{
    MAX_SERVICE_LOG_INSTANCE_BYTES, MAX_SERVICE_LOG_INSTANCE_CHUNKS, MAX_SERVICE_LOG_PROJECT_BYTES,
    MAX_SERVICE_LOG_PROJECT_CHUNKS,
};
use sqlx::PgPool;

// These platform caps are bounded well below `i64::MAX`; the narrow casts
// keep the SQL arithmetic in the same type as PostgreSQL bigint.
#[allow(clippy::cast_possible_wrap)]
const MAX_INSTANCE_BYTES: i64 = MAX_SERVICE_LOG_INSTANCE_BYTES as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_INSTANCE_CHUNKS: i64 = MAX_SERVICE_LOG_INSTANCE_CHUNKS as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_PROJECT_BYTES: i64 = MAX_SERVICE_LOG_PROJECT_BYTES as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_PROJECT_CHUNKS: i64 = MAX_SERVICE_LOG_PROJECT_CHUNKS as i64;
const MAX_PROJECT_EPOCHS: i32 = 128;
const RETENTION_HOURS: i64 = 24;
const PRESSURE_HIGH_NUMERATOR: i64 = 7;
const PRESSURE_HIGH_DENOMINATOR: i64 = 8;
const PRESSURE_LOW_NUMERATOR: i64 = 3;
const PRESSURE_LOW_DENOMINATOR: i64 = 4;

mod append;
mod append_helpers;
mod maintenance_deletion;
mod maintenance_project;
mod maintenance_projects;
mod maintenance_queries;

pub use append_helpers::{
    ChunkRow, EpochRow, InstanceRow, InstanceUsageRow, UsageRow, ensure_current_lease, storage,
    stream_name, validate_lease,
};

pub use maintenance_deletion::{
    delete_gc_log_epochs, delete_log_chunks, has_more_maintenance_work, load_gc_log_epochs,
    maintenance_storage, maintenance_usize, pressure_above_low_instance,
    pressure_above_low_project, pressure_needed_instance, pressure_needed_project,
};
pub use maintenance_queries::{
    DeletedChunks, MaintenanceChunk, MaintenanceEpoch, MaintenanceProjectUsage, MaintenanceUsage,
    load_instance_epochs, load_instance_usage, load_log_chunks, lock_retention_scopes,
    same_chunk_identity,
};

/// `PostgreSQL` adapter for the worker-owned application log append port.
#[derive(Clone)]
pub struct PostgresGatewayServiceLogStore {
    pool: PgPool,
}

impl PostgresGatewayServiceLogStore {
    /// Creates an append adapter over the worker-authorized pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
