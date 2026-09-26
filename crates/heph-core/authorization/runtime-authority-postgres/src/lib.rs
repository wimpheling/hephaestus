//! `PostgreSQL` persistence for immutable runtime authority snapshots and
//! hash-only short-lived sessions.

mod runtime_authority;

pub use runtime_authority::{
    PgGatewayRuntimeAuthorityIssuer, PgGatewayRuntimeSessionRepository, PgRuntimeSessionRepository,
};
