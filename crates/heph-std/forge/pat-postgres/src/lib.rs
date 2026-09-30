//! PostgreSQL-backed developer personal access token issuance and lifecycle.
//!
//! The service generates bearer material in memory, persists only its
//! domain-separated verifier, and exposes verifier-free metadata to callers.

mod pat_postgres;

pub use pat_postgres::{
    AuthenticatedPersonalAccessToken, CreatePersonalAccessToken, IssuedPersonalAccessToken,
    PersonalAccessTokenServiceError, PostgresPersonalAccessTokenService, RotatePersonalAccessToken,
};
