//! Authorized `PostgreSQL` reads for persistent service log epoch metadata.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{
    PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction,
    begin_repeatable_read_actor_transaction,
};
use gateway_domain::{
    GatewayServiceLogProjectMetadata, GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata,
    GatewayServiceLogReadPage, GatewayServiceLogReadRecord, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope, MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_READ_PAGE_BYTES,
};
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use time::OffsetDateTime;
use vm_trait::LogStream;

mod reader;
mod reader_queries;
mod reader_types;

use reader_queries::{fetch_metadata, history_incomplete, payload_records, select_sequences};
use reader_types::{CandidateRow, LogMetadataRow, PayloadRow, ProjectUsageRow};

pub use reader::PostgresGatewayServiceLogReader;
pub use reader_types::GatewayServiceLogReaderError;
