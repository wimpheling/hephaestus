//! Compatibility re-export for provider-neutral secret application contracts.
//!
//! `PostgreSQL` persistence implementations live in `secret-postgres`; this
//! crate deliberately contains no database, encryption, or transport code.
pub use secret_application::*;
