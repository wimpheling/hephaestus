//! `PostgreSQL` installation authority for repository-declared HTTP gateways.
//!
//! A caller supplies the exact repository manifest bytes from the release it
//! is installing. This adapter parses that source before opening its
//! transaction, authorizes project management in that transaction, and then
//! atomically installs only immutable declaration revisions and their routes.
//! It deliberately does not open a listener or derive provider configuration.

use agent_config::Diagnostic;
use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use gateway_domain::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayInvocationOutcome,
    GatewayInvocationRecorder, GatewayLimits, GatewayRouteBinding, GatewayRouteResolver,
};
use gateway_domain::{GatewayId, GatewayRevisionId};
#[cfg(test)]
use http::Method;
use identity_domain::AuthenticatedIdentity;
#[cfg(test)]
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope,
};
use release_domain::ReleaseId;
use runtime_authority::{GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::VmMount;

mod gateway_mailbox_payload;
mod gateway_mailbox_publisher;
mod release_artifacts;
mod release_resolver;
mod route_authority;
mod runtime_contract;
mod service_execution;
mod service_failure;
mod service_launch;
mod service_log_reader;
mod service_logs;
pub(crate) mod service_ownership;
mod service_targets;
mod service_vm;
pub(crate) mod ui_browser;

#[cfg(test)]
use gateway_domain::{Exposure, HttpMethod};
#[cfg(test)]
use gateway_mailbox_payload::validate_gateway_mailbox_payload;
pub use gateway_mailbox_publisher::{
    GatewayMailboxPublicationRequest, GatewayMailboxPublicationResult,
    PostgresGatewayMailboxPublisher,
};
#[cfg(test)]
use installer_declaration::{exposure_name, installation_hash, method_name, parse_manifest};
pub(crate) use management_configuration_validation::validate_secret_selections;
pub(crate) use management_helpers::{
    binding_command_key, binding_payload_hash, configure_payload_hash, load_mailbox_binding,
    secret_selection_hash, valid_gateway_producer, valid_gateway_slot, valid_header_name,
};
pub use management_models::{
    ConfigureGatewayRequest, ConfigureGatewayResult, GatewayConfigureError, GatewayIngressSummary,
    GatewayMailboxBindingSummary, GatewayMailboxPublicationSummary, GatewayManagementError,
    GatewayManagementRevision, GatewayManagementRoute, GatewayManagementSummary, GatewayPage,
    GatewaySecretSelection,
};
pub(crate) use management_rows::{
    ConfigureCommandRow, ConfigureRevisionRow, ConfigureRouteRow, GatewayIngressRow,
    GatewayMailboxBindingCommandRow, GatewayMailboxBindingRow, GatewayMailboxBindingTargetRow,
    GatewayMailboxPublicationManagementRow, GatewayRevisionRow, GatewayRouteRow, GatewaySummaryRow,
};
pub(crate) use release_artifacts::gateway_release_artifacts;
pub use release_resolver::PostgresGatewayReleaseResolver;
pub(crate) use route_authority::AcceptedInvocationRow;
#[cfg(test)]
use route_authority::active_route;
use route_authority::{canonical_request_path, route_matches};
pub(crate) use runtime_contract::{GatewayNetwork, GatewayRuntimeContract};
pub use service_execution::PostgresGatewayExecutionTargetResolver;
pub use service_failure::PostgresGatewayServiceFailureStore;
pub use service_launch::PostgresGatewayServiceLaunchResolver;
pub use service_log_reader::{GatewayServiceLogReaderError, PostgresGatewayServiceLogReader};
pub use service_logs::PostgresGatewayServiceLogStore;
pub use service_ownership::PostgresGatewayServiceOwnership;
pub use service_targets::PostgresGatewayServiceTargets;
pub(crate) use service_vm::service_vm_spec;

const SERVICE_RECOVERY_BATCH_SIZE: i64 = 128;

/// Private worker adapter from authoritative gateway rows to the edge ports.
///
/// It never trusts a route identifier supplied by Caddy: resolution starts with
/// the canonical request path and selects only an enabled route of the active
/// immutable revision. Invocation rows retain correlation/lifecycle evidence
/// only; payloads remain at the private HTTP boundary.
#[derive(Clone)]
pub struct PostgresGatewayEdgeAuthority {
    pool: PgPool,
    limits: GatewayLimits,
    runtime_authority: Option<Arc<dyn GatewayRuntimeAuthorityIssuer>>,
    session_ttl: Duration,
}

impl PostgresGatewayEdgeAuthority {
    /// Creates the worker-side route and invocation adapter with explicit
    /// bounded HTTP limits.
    #[must_use]
    pub const fn new(pool: PgPool, limits: GatewayLimits) -> Self {
        Self {
            pool,
            limits,
            runtime_authority: None,
            session_ttl: Duration::from_secs(30),
        }
    }

    /// Attaches the generic gateway-session issuer used by a production
    /// dispatcher. The basic constructor remains useful for reconciliation
    /// workers that never accept public traffic.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when the requested session lifetime
    /// is zero and therefore cannot form a bounded session identity.
    pub fn with_runtime_authority(
        mut self,
        runtime_authority: Arc<dyn GatewayRuntimeAuthorityIssuer>,
        session_ttl: Duration,
    ) -> Result<Self, GatewayEdgeError> {
        if session_ttl.is_zero() {
            return Err(GatewayEdgeError::Unavailable);
        }
        self.runtime_authority = Some(runtime_authority);
        self.session_ttl = session_ttl;
        Ok(self)
    }

    /// Reconstructs the complete enabled desired route set from `PostgreSQL`.
    /// The returned revision is a deterministic digest of the authoritative
    /// active route set; no provider configuration becomes authoritative.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when authoritative rows cannot be
    /// read or contain an invalid persisted HTTP vocabulary.
    pub async fn desired_configuration(
        &self,
    ) -> Result<GatewayDesiredConfiguration, GatewayEdgeError> {
        let routes = self.active_routes().await?;
        Ok(GatewayDesiredConfiguration {
            revision: desired_configuration_revision(&routes),
            routes,
        })
    }

    async fn reject_invocation(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError> {
        let _: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'rejected')")
            .bind(invocation_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(())
    }

    async fn create_gateway_secret_leases(
        &self,
        invocation_id: Uuid,
        runtime_session_id: Uuid,
    ) -> Result<(), GatewayEdgeError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let invocation: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT gateway_id, gateway_revision_id, gateway_route_id, outcome
               FROM gateway_invocations
              WHERE id = $1
              FOR UPDATE",
        )
        .bind(invocation_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some((gateway_id, revision_id, route_id, outcome)) = invocation else {
            return Err(GatewayEdgeError::Unavailable);
        };
        if outcome != "accepted" {
            return Err(GatewayEdgeError::Unavailable);
        }
        let session: Option<(Uuid, Uuid, Uuid, String, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, invocation_id, gateway_revision_id, status, expires_at
               FROM gateway_runtime_authority_sessions
              WHERE id = $1
                AND invocation_id = $2
                AND gateway_id = $3
                AND gateway_revision_id = $4
                AND status IN ('pending_handoff', 'active')
                AND expires_at > now()
              FOR UPDATE",
        )
        .bind(runtime_session_id)
        .bind(invocation_id)
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some((session_id, session_invocation_id, session_revision_id, _status, expires_at)) =
            session
        else {
            return Err(GatewayEdgeError::Unavailable);
        };
        // The invocation row is locked before the session row, matching
        // terminal completion and repository lifecycle cleanup.
        sqlx::query(
            "INSERT INTO gateway_secret_leases
                 (id, runtime_session_id, invocation_id, binding_id,
                  secret_version_id, rule_id, status, expires_at)
             SELECT gen_random_uuid(), $1, $2, binding.id,
                    binding.secret_version_id, rule.id, 'active', $3
             FROM gateway_brokered_secret_rules AS rule
             JOIN gateway_secret_bindings AS binding
               ON binding.id = rule.binding_id
              AND binding.gateway_revision_id = $5
             WHERE rule.gateway_revision_id = $5
               AND rule.gateway_route_id = $4
               AND binding.status = 'active'
             ON CONFLICT (invocation_id, rule_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(session_invocation_id)
        .bind(expires_at)
        .bind(route_id)
        .bind(session_revision_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)
    }

    /// Terminalizes one bounded batch of abandoned host-mediated service
    /// invocations. The caller owns scheduling repeated batches.
    ///
    /// # Errors
    ///
    /// Returns an unavailable error when the authoritative recovery query or
    /// terminal transition cannot be completed.
    pub async fn recover_abandoned_service_invocations(
        &self,
        now: OffsetDateTime,
    ) -> Result<u64, GatewayEdgeError> {
        let Ok(ttl) = time::Duration::try_from(self.session_ttl) else {
            return Err(GatewayEdgeError::Unavailable);
        };
        let Some(cutoff) = now.checked_sub(ttl) else {
            return Ok(0);
        };
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let candidates = recovery_candidates(&mut transaction, cutoff, now).await?;
        let mut processed = 0;
        for invocation_id in candidates {
            let eligible =
                recovery_candidate_is_eligible(&mut transaction, invocation_id, now, cutoff)
                    .await?;
            if !eligible {
                continue;
            }
            let changed: bool =
                sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'timed_out')")
                    .bind(invocation_id)
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|_| GatewayEdgeError::Unavailable)?;
            if changed {
                processed += 1;
            }
        }
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(processed)
    }
    async fn finish_accepted_invocation(
        &self,
        invocation_id: Uuid,
        request_id: Uuid,
        accepted: AcceptedInvocationRow,
    ) -> Result<Uuid, GatewayEdgeError> {
        let Some(issuer) = &self.runtime_authority else {
            if accepted.handler_contract == "http.service.v1" {
                warn_post_admission_setup(request_id, invocation_id, "runtime_authority_missing");
                self.reject_invocation(invocation_id).await?;
                return Err(GatewayEdgeError::Unavailable);
            }
            return Ok(invocation_id);
        };
        let issued_at = OffsetDateTime::now_utc();
        let Ok(ttl) = time::Duration::try_from(self.session_ttl) else {
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        let Some(expires_at) = issued_at.checked_add(ttl) else {
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        let request = GatewayRuntimeSessionRequest {
            invocation_id: capability_domain::GatewayInvocationId::from_uuid(invocation_id),
            gateway_id: accepted.gateway_id,
            gateway_revision_id: accepted.gateway_revision_id,
            issued_at,
            expires_at,
        };
        let issued_result = if accepted.handler_contract == "http.service.v1" {
            issuer.issue_gateway_service(request).await
        } else {
            issuer.issue_gateway(request).await
        };
        let Ok(issued_session) = issued_result else {
            warn_post_admission_setup(
                request_id,
                invocation_id,
                "runtime_authority_issuance_failed",
            );
            self.reject_invocation(invocation_id).await?;
            return Err(GatewayEdgeError::Unavailable);
        };
        if let Err(error) = self
            .create_gateway_secret_leases(invocation_id, issued_session.id.as_uuid())
            .await
        {
            warn_post_admission_setup(request_id, invocation_id, "secret_lease_setup_failed");
            let _ = self
                .completed(invocation_id, GatewayInvocationOutcome::Rejected)
                .await;
            return Err(error);
        }
        Ok(invocation_id)
    }
}

fn warn_post_admission_setup(request_id: Uuid, invocation_id: Uuid, category: &'static str) {
    tracing::warn!(
        target: "gateway_postgres::ui_post_admission",
        request_id = %request_id,
        invocation_id = %invocation_id,
        category,
        "gateway post-admission setup failed"
    );
}

async fn recovery_candidates(
    transaction: &mut Transaction<'_, Postgres>,
    cutoff: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<Vec<Uuid>, GatewayEdgeError> {
    sqlx::query_scalar(
        "SELECT invocation.id
           FROM gateway_invocations AS invocation
           JOIN gateway_revisions AS revision
             ON revision.id = invocation.gateway_revision_id
            AND revision.gateway_id = invocation.gateway_id
           LEFT JOIN gateway_runtime_authority_sessions AS session
             ON session.invocation_id = invocation.id
            AND session.admission_mode = 'host_mediated'
           LEFT JOIN gateway_service_instances AS instance
             ON instance.id = invocation.service_instance_id
            AND instance.gateway_id = invocation.gateway_id
            AND instance.revision_id = invocation.gateway_revision_id
           CROSS JOIN LATERAL (
                SELECT clock_timestamp() AS database_now
           ) AS clock
          WHERE invocation.outcome = 'accepted'
            AND revision.handler_contract = 'http.service.v1'
            AND (
                instance.id IS NULL
                OR instance.fencing_token IS DISTINCT FROM
                   invocation.service_instance_fencing_token
                OR instance.state NOT IN ('ready', 'draining')
                OR instance.lease_expires_at <= clock.database_now
                OR
                session.status IN ('expired', 'revoked')
                OR (session.status = 'active' AND session.expires_at <= $2)
                OR (session.id IS NULL AND invocation.accepted_at <= $1)
            )
          ORDER BY invocation.id
          LIMIT $3
          FOR UPDATE OF invocation SKIP LOCKED",
    )
    .bind(cutoff)
    .bind(now)
    .bind(SERVICE_RECOVERY_BATCH_SIZE)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)
}

async fn recovery_candidate_is_eligible(
    transaction: &mut Transaction<'_, Postgres>,
    invocation_id: Uuid,
    now: OffsetDateTime,
    cutoff: OffsetDateTime,
) -> Result<bool, GatewayEdgeError> {
    sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1
              FROM gateway_invocations AS invocation
              JOIN gateway_revisions AS revision
                ON revision.id = invocation.gateway_revision_id
               AND revision.gateway_id = invocation.gateway_id
              LEFT JOIN gateway_runtime_authority_sessions AS session
                ON session.invocation_id = invocation.id
               AND session.admission_mode = 'host_mediated'
              LEFT JOIN gateway_service_instances AS instance
                ON instance.id = invocation.service_instance_id
               AND instance.gateway_id = invocation.gateway_id
               AND instance.revision_id = invocation.gateway_revision_id
              CROSS JOIN LATERAL (
                   SELECT clock_timestamp() AS database_now
              ) AS clock
             WHERE invocation.id = $1
               AND invocation.outcome = 'accepted'
               AND revision.handler_contract = 'http.service.v1'
               AND (
                   instance.id IS NULL
                   OR instance.fencing_token IS DISTINCT FROM
                      invocation.service_instance_fencing_token
                   OR instance.state NOT IN ('ready', 'draining')
                   OR instance.lease_expires_at <= clock.database_now
                   OR
                   session.status IN ('expired', 'revoked')
                   OR (session.status = 'active' AND session.expires_at <= $2)
                   OR (session.id IS NULL AND invocation.accepted_at <= $3)
               )
        )",
    )
    .bind(invocation_id)
    .bind(now)
    .bind(cutoff)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)
}

fn desired_configuration_revision(routes: &[GatewayRouteBinding]) -> GatewayConfigRevision {
    let mut canonical = routes.to_vec();
    canonical.sort_by(|left, right| {
        left.path_prefix
            .cmp(&right.path_prefix)
            .then_with(|| left.route_id.cmp(&right.route_id))
    });
    let mut hash = Sha256::new();
    for route in canonical {
        hash.update(route.route_id.as_bytes());
        hash.update(route.gateway_revision_id.as_bytes());
        hash.update(route.path_prefix.as_bytes());
        for method in route.methods {
            hash.update(method.as_str().as_bytes());
            hash.update([0]);
        }
    }
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // The stable opaque UUID is only a compact carrier for the complete route
    // digest; it never becomes a source of configuration authority.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    GatewayConfigRevision::from_uuid(Uuid::from_bytes(bytes))
}

#[async_trait]
impl GatewayRouteResolver for PostgresGatewayEdgeAuthority {
    async fn resolve(
        &self,
        path_and_query: &str,
    ) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError> {
        let path = canonical_request_path(path_and_query)?;
        let mut candidates = self.active_routes().await?;
        candidates.retain(|route| route_matches(route, path));
        candidates.sort_by(|left, right| {
            right
                .path_prefix
                .len()
                .cmp(&left.path_prefix.len())
                .then_with(|| left.route_id.cmp(&right.route_id))
        });
        Ok(candidates.into_iter().next())
    }
}

#[async_trait]
impl GatewayInvocationRecorder for PostgresGatewayEdgeAuthority {
    async fn accepted(
        &self,
        route: &GatewayRouteBinding,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        let invocation_id = Uuid::new_v4();
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;

        let accepted = self
            .lock_authoritative_route(&mut transaction, route)
            .await?;
        if accepted.handler_contract != "http.v1" && accepted.handler_contract != "http.service.v1"
        {
            tracing::warn!(
                invocation_id = %invocation_id,
                handler_contract = %accepted.handler_contract,
                "gateway invocation has unsupported handler contract"
            );
            return Err(GatewayEdgeError::Unavailable);
        }

        let service_binding = if accepted.handler_contract == "http.service.v1" {
            Some(
                self.service_admission_binding(&mut transaction, &accepted)
                    .await?,
            )
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO gateway_invocations
                 (id, gateway_id, gateway_revision_id, gateway_route_id,
                  project_id, request_id, outcome,
                  service_instance_id, service_instance_fencing_token)
             SELECT $1, route.gateway_id, route.gateway_revision_id, route.id,
                    route.project_id, $2, 'accepted', $5, $6
               FROM gateway_routes AS route
              WHERE route.id = $3
                AND route.gateway_revision_id = $4",
        )
        .bind(invocation_id)
        .bind(request_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .bind(service_binding.as_ref().map(|binding| binding.0))
        .bind(service_binding.as_ref().map(|binding| binding.1))
        .execute(&mut *transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;

        self.finish_accepted_invocation(invocation_id, request_id, accepted)
            .await
    }

    async fn accepted_ui(
        &self,
        route: &GatewayRouteBinding,
        authority: &gateway_domain::UiGatewayAuthority,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        ui_browser::accept_ui_invocation(self, route, authority, request_id).await
    }

    async fn completed(
        &self,
        invocation_id: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        let completed: bool = sqlx::query_scalar("SELECT gateway_invocation_complete($1, $2)")
            .bind(invocation_id)
            .bind(outcome_name(outcome))
            .fetch_one(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if completed {
            Ok(())
        } else {
            Err(GatewayEdgeError::Unavailable)
        }
    }
}

/// Verified artifact metadata selected for one immutable gateway release.
#[derive(Debug, Clone)]
pub struct GatewayReleaseArtifact {
    /// Safe release-relative path.
    pub path: String,
    /// Closed persisted artifact kind.
    pub kind: GatewayReleaseArtifactKind,
    /// Exact Unix mode declared by the release.
    pub mode: u32,
    /// SHA-256 digest of the canonical object.
    pub content_hash: [u8; 32],
    /// Exact canonical object length.
    pub size_bytes: u64,
    /// Opaque canonical object-store key.
    pub storage_key: Uuid,
}

/// Closed artifact kinds accepted for a gateway release tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayReleaseArtifactKind {
    /// Guest-executable regular file.
    Executable,
    /// Non-executable regular file.
    File,
    /// Non-executable manifest file.
    Manifest,
}

/// Host-owned lifecycle boundary for a materialized gateway `/release` tree.
///
/// The resolver supplies only exact persisted metadata. Implementations own
/// object-store I/O, safe filesystem construction, and cleanup after the VM
/// is destroyed.
pub trait GatewayReleaseMaterializer: Send + Sync {
    /// Builds fresh read-only release and parameter mounts for this invocation.
    ///
    /// # Errors
    ///
    /// Returns a safe edge error when the exact release tree cannot be built.
    fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[GatewayReleaseArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, GatewayEdgeError>;
    /// Removes the materialized tree after provider cleanup.
    ///
    /// # Errors
    ///
    /// Returns a safe edge error when the release tree cannot be removed.
    fn destroy(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError>;
}

const fn outcome_name(outcome: GatewayInvocationOutcome) -> &'static str {
    match outcome {
        GatewayInvocationOutcome::Completed => "completed",
        GatewayInvocationOutcome::Failed => "failed",
        GatewayInvocationOutcome::TimedOut => "timed_out",
        GatewayInvocationOutcome::Rejected => "rejected",
    }
}

/// Authorized `PostgreSQL` management query and lifecycle adapter.
#[derive(Clone)]
pub struct PostgresGatewayManagement {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

mod installer_command_ledger;
mod installer_commands;
mod installer_declaration;
mod management_commands;
mod management_configuration;
mod management_configuration_validation;
mod management_helpers;
mod management_models;
mod management_queries;
mod management_rows;

impl PostgresGatewayManagement {
    /// Creates the management adapter over the control-plane connection pool.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

    async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), GatewayManagementError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await
            .map_err(|_| GatewayManagementError::Unavailable)?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            Err(GatewayManagementError::Denied)
        }
    }
}

/// Trusted request to install all valid declarations in one exact manifest.
#[derive(Debug, Clone)]
pub struct InstallGatewayManifest {
    /// Project which owns the repository and durable gateway identities.
    pub project_id: ProjectId,
    /// Repository containing the manifest.
    pub repository_id: RepositoryId,
    /// Optional immutable release whose source supplied this manifest.
    pub release_id: Option<ReleaseId>,
    /// Exact bytes from the repository-root `heph.gateways.toml` file.
    pub manifest: Vec<u8>,
}

/// One installed stable gateway and its selected immutable revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledGateway {
    /// Durable repository-scoped gateway identity.
    pub gateway_id: GatewayId,
    /// Immutable declaration selected as active by this installation.
    pub revision_id: GatewayRevisionId,
}

/// Result of atomically installing one manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallGatewayManifestResult {
    /// Gateways installed in source declaration order.
    pub gateways: Vec<InstalledGateway>,
}

/// Immutable source coordinates for one published gateway release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedGatewayRelease {
    /// Published release identity.
    pub release_id: ReleaseId,
    /// Project owning the release repository.
    pub project_id: ProjectId,
    /// Repository containing the release source.
    pub repository_id: RepositoryId,
    /// Exact source commit recorded by the release.
    pub source_commit: String,
}

/// Safe installation failure. Manifest diagnostics are intentionally retained
/// for trusted repository feedback; no database details are exposed.
#[derive(Debug, thiserror::Error)]
pub enum GatewayInstallError {
    /// The manifest did not pass the repository declaration contract.
    #[error("invalid gateway manifest")]
    InvalidManifest {
        /// Parser diagnostics suitable for repository feedback.
        diagnostics: Vec<Diagnostic>,
    },
    /// The caller cannot manage the target project.
    #[error("gateway installation is not authorized")]
    AuthorizationDenied,
    /// The repository, release, or project boundary is unavailable.
    #[error("gateway installation target is unavailable")]
    Unavailable,
    /// The idempotency occurrence already belongs to another release.
    #[error("gateway installation idempotency key conflicts")]
    Conflict,
    /// The durable gateway declaration could not be written.
    #[error("gateway installation persistence failed")]
    Persistence(#[from] sqlx::Error),
}

/// `PostgreSQL` control-plane service for immutable gateway installation.
#[derive(Clone)]
pub struct PostgresGatewayInstaller {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl PostgresGatewayInstaller {
    /// Creates an installer over the control-plane pool.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        time::Duration,
    };

    #[test]
    fn parses_only_validated_repository_gateway_source() {
        let source = br#"
version = 1

[[gateways]]
name = "telegram"
agent_name = "telegram-handler"
handler_contract = "http.v1"
exposure = "public"
parameters = {}

[[gateways.routes]]
path = "/telegram"
methods = ["POST"]
"#;
        let parsed = parse_manifest(source).expect("valid repository source");
        assert_eq!(parsed.gateways.len(), 1);
        assert!(parse_manifest(b"version = 2").is_err());
        assert!(parse_manifest(&vec![b'x'; 1_048_577]).is_err());
    }

    #[test]
    fn serializes_only_the_bounded_http_vocabulary() {
        assert_eq!(method_name(HttpMethod::Patch), "PATCH");
        assert_eq!(
            exposure_name(Exposure::HephAuthenticated),
            "heph_authenticated"
        );
    }

    #[test]
    fn mailbox_binding_input_is_strictly_bounded_before_persistence() {
        assert!(valid_gateway_slot("recipe-events_2"));
        assert!(!valid_gateway_slot("Recipe-events"));
        assert!(!valid_gateway_slot("-recipe-events"));
        assert!(!valid_gateway_slot(&"a".repeat(65)));
        assert!(valid_gateway_producer("telegram-relay/v1"));
        assert!(!valid_gateway_producer(" producer"));
        assert!(!valid_gateway_producer("producer\nnext"));
        assert!(!valid_gateway_producer(&"p".repeat(129)));
    }

    #[test]
    fn publication_inspection_maps_only_joined_lifecycle_provenance() {
        let now = OffsetDateTime::now_utc();
        let snapshot_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let summary =
            GatewayMailboxPublicationSummary::from(GatewayMailboxPublicationManagementRow {
                id: Uuid::new_v4(),
                invocation_id: Uuid::new_v4(),
                gateway_revision_id: Uuid::new_v4(),
                binding_id: Some(Uuid::new_v4()),
                grant_id: Some(Uuid::new_v4()),
                mailbox_id: Some(Uuid::new_v4()),
                event_id: Some(Uuid::new_v4()),
                slot_key: String::from("recipe-events"),
                outcome: String::from("accepted"),
                accepted_at: now,
                settled_at: now,
                authorization_snapshot_id: Some(snapshot_id),
                snapshot_binding_ordinal: Some(3),
                delivery_disposition: Some(String::from("delivered")),
                delivery_attempt_count: Some(1),
                delivery_terminal_at: Some(now),
                delivery_attempt_id: Some(attempt_id),
                run_id: Some(run_id),
                run_state: Some(String::from("succeeded")),
                run_outcome: Some(String::from("succeeded")),
            });
        assert_eq!(summary.authorization_snapshot_id, Some(snapshot_id));
        assert_eq!(summary.snapshot_binding_ordinal, Some(3));
        assert_eq!(summary.delivery_disposition.as_deref(), Some("delivered"));
        assert_eq!(summary.delivery_attempt_id, Some(attempt_id));
        assert_eq!(summary.run_id, Some(run_id));
        assert_eq!(summary.run_outcome.as_deref(), Some("succeeded"));
    }

    #[test]
    fn gateway_mailbox_payload_validation_preserves_an_empty_body() {
        let body = Vec::new();
        let hash: [u8; 32] = Sha256::digest(&body).into();
        let envelope = MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/empty").expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(BodyReferenceId::new(), 0, hash).expect("zero body"),
                None,
                Some(String::from("identity")),
            )
            .expect("content"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope");
        let request = GatewayMailboxPublicationRequest {
            runtime_session_id: Uuid::new_v4(),
            invocation_id: Uuid::new_v4(),
            slot_key: String::from("recipe-events"),
            deduplication_key: DeduplicationKey::parse("empty-body").expect("key"),
            envelope,
            encoded_body: body,
            decoded_length: 0,
        };

        assert!(validate_gateway_mailbox_payload(&request).is_ok());
    }

    fn edge_limits() -> GatewayLimits {
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 1024,
            execution_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn edge_resolution_selects_the_longest_exact_path_segment() {
        let short = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let nested = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram/updates"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        assert!(route_matches(&short, "/gateway/telegram"));
        assert!(route_matches(&nested, "/gateway/telegram/updates"));
        assert!(!route_matches(&short, "/gateway/telegram-bot"));
        assert_eq!(
            canonical_request_path("/gateway/telegram/updates?offset=1").expect("path"),
            "/gateway/telegram/updates"
        );
    }

    #[test]
    fn edge_route_conversion_rejects_unknown_persisted_method() {
        let row = route_authority::ActiveRouteRow {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path: String::from("/telegram"),
            methods: vec![String::from("CONNECT")],
            exposure: String::from("public"),
        };
        assert!(active_route(row, edge_limits()).is_err());
    }

    #[test]
    fn desired_configuration_revision_is_order_independent_and_tracks_cutover() {
        let first = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("first"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let second = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("second"),
            methods: BTreeSet::from([Method::GET]),
            limits: edge_limits(),
        };
        assert_eq!(
            desired_configuration_revision(&[first.clone(), second.clone()]),
            desired_configuration_revision(&[second.clone(), first.clone()])
        );
        let before_cutover = desired_configuration_revision(&[first.clone(), second.clone()]);
        let replacement = GatewayRouteBinding {
            gateway_revision_id: Uuid::new_v4(),
            ..second
        };
        assert_ne!(
            before_cutover,
            desired_configuration_revision(&[first, replacement])
        );
    }

    #[test]
    fn installation_hash_is_stable_per_release_and_agent() {
        let declaration = [7_u8; 32];
        let release = ReleaseId::new();
        let agent = Uuid::new_v4();
        assert_eq!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(release), agent)
        );
        assert_ne!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(ReleaseId::new()), agent)
        );
        assert_ne!(
            installation_hash(declaration, Some(release), agent),
            installation_hash(declaration, Some(release), Uuid::new_v4())
        );
    }
}
