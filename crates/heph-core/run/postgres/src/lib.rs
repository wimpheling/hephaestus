//! `PostgreSQL` persistence adapter for durable runs.

#[path = "run_postgres/create.rs"]
mod create;
#[path = "run_postgres/errors.rs"]
mod errors;
#[path = "run_postgres/model.rs"]
mod model;
#[path = "run_postgres/operations.rs"]
mod operations;
#[path = "run_postgres/provenance.rs"]
mod provenance;
#[path = "run_postgres/repository.rs"]
mod repository;
mod runtime_catalog;

pub use repository::PgRunRepository;
