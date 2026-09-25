//! `PostgreSQL` implementation of encrypted secret commands and runtime resolution.
#![allow(clippy::wildcard_imports)] // Keep adapter method signatures aligned with the port.

use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{
    AUTHORIZATION_MODEL_VERSION, PostgresMelangeAuthorizer, audit_decision,
    begin_actor_transaction, begin_runtime_transaction,
};
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName as BrokeredHeaderName,
    HttpInjectionLocation,
};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};
use forge_domain::{CommitSha, GitRef, ProjectId};
use gateway_domain::{GatewayError, GatewayInboundSecretResolver, InboundGatewaySecretRule};
use heph_secret::SecretMountProvider;
use http::{HeaderName, HeaderValue};
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use release_domain::{AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_application::*;
use secret_domain::*;
use secret_store::{EncryptedSecretVersion, EncryptedStore, KeyProvider, VersionContext};
use serde_json::json;
use sha2::Digest;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

#[path = "service/authorization.rs"]
mod authorization;
#[path = "service/binding_commands.rs"]
mod binding_commands;
#[path = "service/binding_models.rs"]
mod binding_models;
#[path = "service/binding_persist.rs"]
mod binding_persist;
#[path = "service/binding_policy.rs"]
mod binding_policy;
#[path = "service/binding_prepare.rs"]
mod binding_prepare;
#[path = "service/broker_authorization.rs"]
mod broker_authorization;
#[path = "service/broker_commands.rs"]
mod broker_commands;
#[path = "service/broker_execution.rs"]
mod broker_execution;
#[path = "service/broker_usage.rs"]
mod broker_usage;
#[path = "service/command_helpers.rs"]
mod command_helpers;
#[path = "service/dispatch_leases.rs"]
mod dispatch_leases;
#[path = "service/dispatch_resolution.rs"]
mod dispatch_resolution;
#[path = "service/gateway_ingress.rs"]
mod gateway_ingress;
#[path = "service/grant_accept.rs"]
mod grant_accept;
#[path = "service/grant_commands.rs"]
mod grant_commands;
#[path = "service/lifecycle_commands.rs"]
mod lifecycle_commands;
#[path = "service/runtime_receive.rs"]
mod runtime_receive;
#[path = "service/runtime_versions.rs"]
mod runtime_versions;
#[path = "service/service_traits.rs"]
mod service_traits;

use binding_models::{
    BrokeredRuleBindingRow, CarriedBindingRow, CarriedCapabilityRow, EligibleImportRow,
    RevisionCloneRow, clone_capability_binding,
};
use binding_policy::{
    declared_slot, insert_binding_copy, load_eligible_import, unresolved_required_diagnostics,
    validate_binding_scope, validate_carried_policy, validate_declared_binding,
    validate_import_policy,
};
use broker_usage::{
    broker_request_rule_id, record_https_operation, record_runtime_use, record_runtime_use_tx,
    validate_broker_request,
};
use command_helpers::{
    GrantAcceptanceRow, SecretRotationRow, audit, existing_command, insert_encrypted_version,
    mode_name, normalized_modes, parse_mode, phase_name, record_command, resolve_owner,
    resolve_target,
};
use runtime_versions::{
    BrokeredSecretDenialContextRow, GatewayEncryptedVersionRow, RuntimeLeaseAuthorizationRow,
    RuntimeSessionRow, gateway_encrypted_version, load_runtime_version,
};
#[path = "service/secret_commands.rs"]
mod secret_commands;

/// PostgreSQL-backed encrypted secret command service.
#[derive(Clone)]
pub struct SecretService<K> {
    pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

/// Agent-facing runtime service with a non-bypass authorization pool and a
/// distinct narrow worker pool for exact ciphertext resolution.
#[derive(Clone)]
pub struct SecretRuntimeService<K> {
    authorization_pool: PgPool,
    resolver_pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<PostgresMelangeAuthorizer>,
    mount_provider: Option<Arc<dyn SecretMountProvider>>,
}

/// Host-only resolver for declared inbound gateway webhook-secret rules.
/// It decrypts an exact live invocation lease only long enough to build the
/// edge's non-serializable constant-time matcher.
#[derive(Clone)]
pub struct GatewayIngressSecretResolver<K> {
    resolver_pool: PgPool,
    encrypted_store: EncryptedStore<K>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreAdapterDenialClass {
    AuthenticationDenied,
    AuthorityUnavailable,
    AuthorizationDenied,
    RequestDenied,
    PersistenceFailure,
    SecretResolutionFailure,
    OtherFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreAdapterDenialStage {
    SessionAuthentication,
    LeaseAuthorization,
    RequestAuthorization,
    VersionLoading,
    Decryption,
}

#[derive(Debug, sqlx::FromRow)]
struct BrokeredHttpsRuleSnapshotRow {
    rule_id: Uuid,
    destination_origin: String,
    header_name: String,
    header_prefix: Option<String>,
}

impl PreAdapterDenialStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SessionAuthentication => "session-authentication",
            Self::LeaseAuthorization => "lease-authorization",
            Self::RequestAuthorization => "request-authorization",
            Self::VersionLoading => "version-loading",
            Self::Decryption => "decryption",
        }
    }
}

impl PreAdapterDenialClass {
    const fn from_error(error: &SecretServiceError) -> Self {
        match error {
            SecretServiceError::RuntimeAuthenticationDenied => Self::AuthenticationDenied,
            SecretServiceError::Unavailable => Self::AuthorityUnavailable,
            SecretServiceError::AuthorizationDenied => Self::AuthorizationDenied,
            SecretServiceError::BrokerRequestDenied => Self::RequestDenied,
            SecretServiceError::Persistence => Self::PersistenceFailure,
            SecretServiceError::Encryption(_) => Self::SecretResolutionFailure,
            _ => Self::OtherFailure,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::AuthenticationDenied => "authentication_denied",
            Self::AuthorityUnavailable => "authority_unavailable",
            Self::AuthorizationDenied => "authorization_denied",
            Self::RequestDenied => "request_denied",
            Self::PersistenceFailure => "persistence_failure",
            Self::SecretResolutionFailure => "secret_resolution_failure",
            Self::OtherFailure => "other_failure",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PostgresMelangeAuthorizer, PreAdapterDenialClass, PreAdapterDenialStage,
        SecretRuntimeService, SecretServiceError,
    };
    use heph_secret::{
        EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretMountProvider,
        SecretRuntimeError,
    };
    use secret_store::{EncryptedStore, LocalKeyProvider, SecretStoreError};
    use sqlx::postgres::PgPoolOptions;
    use std::{collections::BTreeSet, sync::Arc};
    use uuid::Uuid;

    #[derive(Debug)]
    struct TestMountProvider;

    impl SecretMountProvider for TestMountProvider {
        fn validate_config(
            &self,
            _config: &EphemeralSecretConfig,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn materialize(
            &self,
            _config: &EphemeralSecretConfig,
            _run_id: runtime_types::RunId,
            _files: Vec<RawSecretFile>,
            _credential: Option<&secret_domain::OpaqueRuntimeCredential>,
        ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
            Err(SecretRuntimeError::InvalidRoot)
        }

        fn discard_materialized(
            &self,
            _config: &EphemeralSecretConfig,
            _opaque_directory: Uuid,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn destroy_confirmed(
            &self,
            _config: &EphemeralSecretConfig,
            _opaque_directory: Uuid,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn reconcile_orphans(
            &self,
            _config: &EphemeralSecretConfig,
            _live_directories: &BTreeSet<String>,
        ) -> Result<usize, SecretRuntimeError> {
            Ok(0)
        }
    }

    fn runtime_service() -> SecretRuntimeService<LocalKeyProvider> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://localhost/hephaestus")
            .expect("lazy PostgreSQL pool");
        let keys =
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])]).expect("test key provider");
        SecretRuntimeService::new(
            pool.clone(),
            pool,
            EncryptedStore::new(keys),
            Arc::new(PostgresMelangeAuthorizer),
        )
    }

    #[tokio::test]
    async fn mount_provider_builder_installs_provider() {
        let runtime = runtime_service().with_mount_provider(Arc::new(TestMountProvider));
        assert!(runtime.mount_provider().is_some());
    }

    #[test]
    fn pre_adapter_denial_classes_are_distinct_and_payload_free() {
        let cases = [
            (
                SecretServiceError::RuntimeAuthenticationDenied,
                "authentication_denied",
            ),
            (SecretServiceError::Unavailable, "authority_unavailable"),
            (
                SecretServiceError::AuthorizationDenied,
                "authorization_denied",
            ),
            (SecretServiceError::BrokerRequestDenied, "request_denied"),
            (SecretServiceError::Persistence, "persistence_failure"),
            (
                SecretServiceError::Encryption(SecretStoreError::Authentication),
                "secret_resolution_failure",
            ),
            (SecretServiceError::InvalidLifecycle, "other_failure"),
        ];
        let mut classes = std::collections::BTreeSet::new();
        for (error, expected) in cases {
            let class = PreAdapterDenialClass::from_error(&error);
            assert_eq!(class.as_str(), expected);
            assert!(classes.insert(class.as_str()));
            assert!(!class.as_str().contains("sentinel"));
            assert!(!class.as_str().contains("password"));
        }
    }

    #[test]
    fn pre_adapter_denial_stages_are_static_and_distinct() {
        let stages = [
            PreAdapterDenialStage::SessionAuthentication,
            PreAdapterDenialStage::LeaseAuthorization,
            PreAdapterDenialStage::RequestAuthorization,
            PreAdapterDenialStage::VersionLoading,
            PreAdapterDenialStage::Decryption,
        ];
        let values = stages.map(PreAdapterDenialStage::as_str);
        assert_eq!(
            values,
            [
                "session-authentication",
                "lease-authorization",
                "request-authorization",
                "version-loading",
                "decryption",
            ]
        );
    }
}
