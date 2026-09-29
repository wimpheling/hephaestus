//! `PostgreSQL` persistence adapter for secret runtime metadata.
#![allow(clippy::wildcard_imports)] // Adapter implementation mirrors the large provider-neutral API.
#![allow(missing_docs)] // Legacy query DTOs are replaced by typed ports incrementally.
#![allow(
    clippy::unused_async,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::struct_excessive_bools
)]
//!
//! Filesystem materialization and encrypted-store/broker effects remain in
//! `secret-runtime` and `secret-service`; this crate owns only SQL queries and
//! the journal transitions around ephemeral mounts.

mod application;
mod application_types;
mod mounts;
mod service;

pub use application::SecretApplication;
pub use application_types::{
    GrantSummary, ImportSummary, Page, PageResult, ProjectAuthority, SecretQueryError,
    SecretSummary,
};
pub use mounts::{PgSecretMountManager, PostgresSecretMountMetadata, initialize_manager};
pub use service::{GatewayIngressSecretResolver, SecretRuntimeService, SecretService};

use application_types::{
    GrantRow, ImportRow, SecretRow, finish_grant_page, finish_import_page, finish_secret_page,
    validate_page,
};

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::{SecretRuntimeService, SecretService, initialize_manager};
    use authz_postgres::PostgresMelangeAuthorizer;
    use heph_secret::EphemeralSecretConfig;
    use secret_store::{EncryptedStore, LocalKeyProvider};
    use sqlx::postgres::PgPoolOptions;
    use std::{path::PathBuf, sync::Arc};

    #[tokio::test]
    async fn initialize_manager_fails_closed_without_mount_provider() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://localhost/hephaestus")
            .expect("lazy PostgreSQL pool");
        let keys =
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])]).expect("test key provider");
        let authorizer = Arc::new(PostgresMelangeAuthorizer);
        let dispatch = SecretService::new(
            pool.clone(),
            EncryptedStore::new(keys.clone()),
            authorizer.clone(),
        );
        let runtime = SecretRuntimeService::new(
            pool.clone(),
            pool.clone(),
            EncryptedStore::new(keys),
            authorizer,
        );
        let result = initialize_manager(
            pool,
            dispatch,
            runtime,
            EphemeralSecretConfig {
                root: PathBuf::from("/invalid/secret-root"),
                require_memory_filesystem: true,
            },
        );
        let error = match result {
            Ok(_) => panic!("missing mount provider must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "run secret operation failed: secret filesystem provider is unavailable"
        );
    }
}
