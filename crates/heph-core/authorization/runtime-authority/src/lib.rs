//! Application contracts for issuing exact, short-lived runtime authority.
//!
//! `PostgreSQL` persists immutable snapshots and hash-only session records. A
//! separate trusted host adapter temporarily retains the encrypted bearer
//! credential until the guest acknowledges the exact issuance generation.

mod error;
mod issuer;
mod models;

pub use error::RuntimeAuthorityError;
pub use issuer::{IssuedRuntimeSession, RuntimeSessionIssuer};
pub use models::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, NewRuntimeSession,
    RuntimeHandoffStore, RuntimeSessionRepository, StoredRuntimeSession,
};
