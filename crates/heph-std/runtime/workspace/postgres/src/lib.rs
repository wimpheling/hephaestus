//! `PostgreSQL` persistence adapter for workspace metadata and results.

mod error;
mod repository;
mod results;
mod rows;

pub use repository::PgWorkspaceMetadataRepository;
