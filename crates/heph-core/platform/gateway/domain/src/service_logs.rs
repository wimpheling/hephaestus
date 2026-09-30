//! Bounded, opt-in application log capture for one service instance.

mod buffer;
mod maintenance;
mod read;

pub use buffer::{
    GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome, GatewayServiceLogStore,
    GatewayServiceLogStoreError, MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_INSTANCE_BYTES,
    MAX_SERVICE_LOG_INSTANCE_CHUNKS, MAX_SERVICE_LOG_PROJECT_BYTES, MAX_SERVICE_LOG_PROJECT_CHUNKS,
    MAX_SERVICE_LOG_QUEUE_BYTES, MAX_SERVICE_LOG_QUEUE_CHUNKS, ServiceLogBufferSnapshot,
    ServiceLogLoss, ServiceLogRecord,
};
pub use maintenance::{
    GatewayServiceLogMaintenance, GatewayServiceLogMaintenanceError,
    GatewayServiceLogMaintenancePolicy, GatewayServiceLogMaintenanceProjectPage,
    GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceProjects,
    GatewayServiceLogMaintenanceReport, MAX_SERVICE_LOG_MAINTENANCE_CHUNKS,
    MAX_SERVICE_LOG_MAINTENANCE_EPOCHS, MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
    MAX_SERVICE_LOG_PROJECT_EPOCHS,
};
pub use read::{
    GatewayServiceLogProjectMetadata, GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata,
    GatewayServiceLogReadPage, GatewayServiceLogReadRecord, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope, MAX_SERVICE_LOG_READ_PAGE_BYTES, MAX_SERVICE_LOG_READ_PAGE_RECORDS,
};
