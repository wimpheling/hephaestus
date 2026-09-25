//! `PostgreSQL` installation authority for repository-declared HTTP gateways.
//!
//! A caller supplies the exact repository manifest bytes from the release it
//! is installing. This adapter parses that source before opening its
//! transaction, authorizes project management in that transaction, and then
//! atomically installs only immutable declaration revisions and their routes.
//! It deliberately does not open a listener or derive provider configuration.

use agent_config::{Diagnostic, RepositoryGatewaysConfig, parse_repository_gateways};
use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use forge_domain::{ProjectId, RepositoryId};
use gateway_domain::{
    Exposure, GatewayDeclaration, GatewayId, GatewayRevisionId, GatewayServiceConfig, HttpMethod,
    ServiceLogCaptureMode, ServiceProbePath,
};
use gateway_domain::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayInvocationOutcome,
    GatewayInvocationRecorder, GatewayLimits, GatewayRouteBinding, GatewayRouteResolver,
};
#[cfg(test)]
use http::Method;
use identity_domain::AuthenticatedIdentity;
#[cfg(test)]
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope,
};
use release_domain::{ParameterName, ParameterValue, ReleaseCommandKey, ReleaseId};
use runtime_authority::{GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
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
use gateway_mailbox_payload::validate_gateway_mailbox_payload;
pub use gateway_mailbox_publisher::{
    GatewayMailboxPublicationRequest, GatewayMailboxPublicationResult,
    PostgresGatewayMailboxPublisher,
};
pub(crate) use management_configuration_validation::validate_secret_selections;
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

/// A bounded cursor request for redacted gateway management projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayPage {
    /// Maximum number of rows returned.
    pub limit: i64,
    /// Strictly older record id in the documented stable order.
    pub after: Option<Uuid>,
}

/// Safe gateway projection for management clients.
#[derive(Debug, Clone)]
pub struct GatewayManagementSummary {
    /// Durable gateway identity.
    pub id: Uuid,
    /// Owning project identity.
    pub project_id: Uuid,
    /// Owning repository identity.
    pub repository_id: Uuid,
    /// Stable declaration name.
    pub name: String,
    /// Current lifecycle.
    pub lifecycle: String,
    /// Exact current immutable revision, when installed.
    pub active_revision_id: Option<Uuid>,
    /// Latest declared HTTP service revision, which may still be pending
    /// readiness and therefore differ from the serving revision.
    pub desired_service_revision_id: Option<Uuid>,
    /// Latest lifecycle/configuration change time.
    pub updated_at: OffsetDateTime,
}

/// Safe immutable revision projection without parameters or secrets.
#[derive(Debug, Clone)]
pub struct GatewayManagementRevision {
    /// Immutable revision identity.
    pub id: Uuid,
    /// Source release identity.
    pub release_id: Option<Uuid>,
    /// Exact published agent identity that produced this revision.
    pub release_agent_id: Option<Uuid>,
    /// Supported handler contract.
    pub handler_contract: String,
    /// Typed loopback service declaration, when this is a persistent service.
    pub service: Option<GatewayServiceConfig>,
    /// Declared exposure policy.
    pub exposure: String,
    /// Symbolic declared secret slot names only.
    pub secret_slots: Vec<String>,
    /// Symbolic mailbox slots declared by the immutable source manifest.
    pub mailbox_slots: Vec<String>,
    /// Immutable creation time.
    pub created_at: OffsetDateTime,
    /// Immutable route intents in this revision.
    pub routes: Vec<GatewayManagementRoute>,
}

/// Safe route intent projection.
#[derive(Debug, Clone)]
pub struct GatewayManagementRoute {
    /// Durable route identity.
    pub id: Uuid,
    /// Canonical path below the reserved namespace.
    pub path: String,
    /// Bounded accepted methods.
    pub methods: Vec<String>,
    /// Whether this declared route is selectable.
    pub enabled: bool,
}

/// Value-free ingress audit projection.
#[derive(Debug, Clone)]
pub struct GatewayIngressSummary {
    /// Invocation identity.
    pub id: Uuid,
    /// Exact selected revision.
    pub gateway_revision_id: Uuid,
    /// Exact selected route.
    pub gateway_route_id: Uuid,
    /// Terminal or pending safe outcome.
    pub outcome: String,
    /// Acceptance time.
    pub accepted_at: OffsetDateTime,
    /// Terminal time, if the invocation completed.
    pub completed_at: Option<OffsetDateTime>,
}

/// Value-free projection of one immutable gateway mailbox binding and its
/// separately revocable grant.
#[derive(Debug, Clone)]
pub struct GatewayMailboxBindingSummary {
    /// Immutable binding identity.
    pub id: Uuid,
    /// Exact gateway revision that declared the slot.
    pub gateway_revision_id: Uuid,
    /// Exact target mailbox.
    pub mailbox_id: Uuid,
    /// Declared symbolic slot key.
    pub slot_key: String,
    /// Stable bound producer identity.
    pub producer_id: String,
    /// Separately revocable grant identity.
    pub grant_id: Uuid,
    /// `active` or `revoked`.
    pub grant_status: String,
    /// Immutable binding creation time.
    pub created_at: OffsetDateTime,
    /// Grant creation time.
    pub granted_at: OffsetDateTime,
    /// Grant revocation time, when revoked.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Value-free correlation from a gateway invocation to one publication
/// settlement. Payload, headers, deduplication keys, and denial detail stay
/// outside management projections.
#[derive(Debug, Clone)]
pub struct GatewayMailboxPublicationSummary {
    /// Immutable publication audit identity.
    pub id: Uuid,
    /// Exact gateway invocation.
    pub invocation_id: Uuid,
    /// Revision selected for the invocation.
    pub gateway_revision_id: Uuid,
    /// Binding used when publication was accepted, if any.
    pub binding_id: Option<Uuid>,
    /// Grant used when publication was accepted, if any.
    pub grant_id: Option<Uuid>,
    /// Mailbox selected by the binding, if any.
    pub mailbox_id: Option<Uuid>,
    /// Accepted logical mailbox event, if any.
    pub event_id: Option<Uuid>,
    /// Declared slot selected by the guest.
    pub slot_key: String,
    /// Redacted `accepted`, `duplicate`, or `denied` result.
    pub outcome: String,
    /// Publication acceptance attempt time.
    pub accepted_at: OffsetDateTime,
    /// Publication settlement time.
    pub settled_at: OffsetDateTime,
    /// Immutable authorization snapshot selected for the invocation.
    pub authorization_snapshot_id: Option<Uuid>,
    /// Exact mailbox binding ordinal copied into that authorization snapshot.
    pub snapshot_binding_ordinal: Option<i32>,
    /// Current or terminal state of the accepted mailbox delivery.
    pub delivery_disposition: Option<String>,
    /// Number of logical delivery attempts so far.
    pub delivery_attempt_count: Option<i32>,
    /// Durable terminal time, when delivery reached a final disposition.
    pub delivery_terminal_at: Option<OffsetDateTime>,
    /// Most recent delivery-attempt identity, when one exists.
    pub delivery_attempt_id: Option<Uuid>,
    /// Run started by that delivery attempt, when one exists.
    pub run_id: Option<Uuid>,
    /// Current lifecycle state of that run.
    pub run_state: Option<String>,
    /// Final run outcome, when the run is terminal.
    pub run_outcome: Option<String>,
}

/// Authorized `PostgreSQL` management query and lifecycle adapter.
#[derive(Clone)]
pub struct PostgresGatewayManagement {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

/// One explicitly selected inbound secret for a configured gateway revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySecretSelection {
    /// Declaration slot receiving this imported secret.
    pub slot_key: String,
    /// Project-owned import authority.
    pub import_id: Uuid,
    /// Exact immutable secret version.
    pub secret_version_id: Uuid,
    /// Declared route receiving the brokered header.
    pub route_path: String,
    /// Header populated by the broker.
    pub header_name: String,
}

/// Runtime values and secret selections for one immutable gateway revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureGatewayRequest {
    /// Gateway whose declared revision is being configured.
    pub gateway_id: Uuid,
    /// Stateless gateways compare this with the active revision. Service
    /// gateways compare it with the latest desired revision, falling back to
    /// the active revision only when no desired candidate exists.
    pub expected_revision_id: Uuid,
    /// Typed values validated against the published agent schema.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Explicit inbound secret selections; no existing grants are copied.
    pub secret_selections: Vec<GatewaySecretSelection>,
}

/// Result of configuring one immutable gateway revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureGatewayResult {
    /// New immutable candidate revision, or the original result on replay.
    pub revision_id: Uuid,
}

/// Safe failures for the narrow gateway configuration operation.
#[derive(Debug, thiserror::Error)]
pub enum GatewayConfigureError {
    /// Caller lacks project/gateway/secret-import authority.
    #[error("gateway configuration is not authorized")]
    Denied,
    /// Selected gateway or revision is absent.
    #[error("gateway configuration target is unavailable")]
    NotFound,
    /// Typed values or secret selections violate the released declaration.
    #[error("gateway configuration request is invalid")]
    InvalidArgument,
    /// The expected active revision has changed.
    #[error("gateway configuration target is stale")]
    Stale,
    /// The same idempotency key was submitted with another payload.
    #[error("gateway configuration idempotency key conflicts")]
    Conflict,
    /// Database operation failed.
    #[error("gateway configuration is unavailable")]
    Persistence(#[from] sqlx::Error),
}

mod management_commands;
mod management_configuration;
mod management_configuration_validation;
mod management_queries;

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

/// Safe failure category for management operations.
#[derive(Debug, thiserror::Error)]
pub enum GatewayManagementError {
    /// Caller lacks the exact project or gateway relation.
    #[error("gateway management is not authorized")]
    Denied,
    /// The selected gateway is hidden or absent.
    #[error("gateway is not found")]
    NotFound,
    /// Request values violate the bounded binding contract.
    #[error("gateway mailbox binding request is invalid")]
    InvalidArgument,
    /// A binding already exists or its grant is no longer active.
    #[error("gateway mailbox binding state conflicts")]
    Conflict,
    /// A storage failure prevented a safe result.
    #[error("gateway management is unavailable")]
    Unavailable,
    /// `PostgreSQL` persistence failed.
    #[error("gateway management persistence failed")]
    Persistence(#[from] sqlx::Error),
}

#[derive(sqlx::FromRow)]
struct ConfigureRevisionRow {
    project_id: Uuid,
    repository_id: Uuid,
    active_revision_id: Option<Uuid>,
    desired_service_revision_id: Option<Uuid>,
    lifecycle: String,
    release_id: Option<Uuid>,
    release_agent_id: Option<Uuid>,
    release_agent_key: Option<String>,
    handler_contract: String,
    service_loopback_port: Option<i32>,
    service_readiness_path: Option<String>,
    service_health_path: Option<String>,
    service_log_capture_mode: String,
    exposure: String,
    secret_slots: Vec<String>,
    mailbox_slots: Vec<String>,
    parameter_schema: Option<serde_json::Value>,
    release_state: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConfigureRouteRow {
    path: String,
    methods: Vec<String>,
    enabled: bool,
}

#[derive(sqlx::FromRow)]
struct ConfigureCommandRow {
    gateway_id: Uuid,
    expected_revision_id: Uuid,
    payload_hash: Vec<u8>,
    actor_id: Uuid,
    result_revision_id: Option<Uuid>,
}

fn valid_header_name(value: &str) -> bool {
    http::HeaderName::from_bytes(value.as_bytes()).is_ok()
}

fn configure_payload_hash(
    gateway_id: Uuid,
    expected_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    parameter_hash: &[u8],
    selections: &[GatewaySecretSelection],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.gateway.configure.v1\0");
    digest.update(gateway_id.as_bytes());
    digest.update(expected_revision_id.as_bytes());
    digest.update(release_id.as_bytes());
    digest.update(release_agent_id.as_bytes());
    digest.update(parameter_hash);
    let mut ordered = selections.to_vec();
    ordered.sort_by(|left, right| {
        left.slot_key
            .cmp(&right.slot_key)
            .then_with(|| left.route_path.cmp(&right.route_path))
            .then_with(|| left.header_name.cmp(&right.header_name))
    });
    for selection in ordered {
        digest.update(selection.slot_key.as_bytes());
        digest.update([0]);
        digest.update(selection.import_id.as_bytes());
        digest.update(selection.secret_version_id.as_bytes());
        digest.update(selection.route_path.as_bytes());
        digest.update([0]);
        digest.update(selection.header_name.as_bytes());
        digest.update([0]);
    }
    digest.finalize().into()
}

fn secret_selection_hash(selection: &GatewaySecretSelection, revision_id: Uuid) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.gateway.secret-selection.v1\0");
    digest.update(revision_id.as_bytes());
    digest.update(selection.slot_key.as_bytes());
    digest.update(selection.import_id.as_bytes());
    digest.update(selection.secret_version_id.as_bytes());
    digest.update(selection.route_path.as_bytes());
    digest.update(selection.header_name.as_bytes());
    digest.finalize().into()
}

fn valid_gateway_slot(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn valid_gateway_producer(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

#[derive(sqlx::FromRow)]
struct GatewaySummaryRow {
    id: Uuid,
    project_id: Uuid,
    repository_id: Uuid,
    name: String,
    lifecycle: String,
    active_revision_id: Option<Uuid>,
    desired_service_revision_id: Option<Uuid>,
    updated_at: OffsetDateTime,
}
impl From<GatewaySummaryRow> for GatewayManagementSummary {
    fn from(row: GatewaySummaryRow) -> Self {
        Self {
            id: row.id,
            project_id: row.project_id,
            repository_id: row.repository_id,
            name: row.name,
            lifecycle: row.lifecycle,
            active_revision_id: row.active_revision_id,
            desired_service_revision_id: row.desired_service_revision_id,
            updated_at: row.updated_at,
        }
    }
}
#[derive(sqlx::FromRow)]
struct GatewayRevisionRow {
    id: Uuid,
    release_id: Option<Uuid>,
    release_agent_id: Option<Uuid>,
    handler_contract: String,
    service_loopback_port: Option<i32>,
    service_readiness_path: Option<String>,
    service_health_path: Option<String>,
    service_log_capture_mode: String,
    exposure: String,
    secret_slots: Vec<String>,
    mailbox_slots: Vec<String>,
    created_at: OffsetDateTime,
}

impl GatewayRevisionRow {
    fn service_config(&self) -> Result<Option<GatewayServiceConfig>, GatewayManagementError> {
        service_config_from_columns(
            self.service_loopback_port,
            self.service_readiness_path.clone(),
            self.service_health_path.clone(),
            &self.service_log_capture_mode,
        )
    }
}

fn service_config_from_columns(
    loopback_port: Option<i32>,
    readiness_path: Option<String>,
    health_path: Option<String>,
    log_capture_mode: &str,
) -> Result<Option<GatewayServiceConfig>, GatewayManagementError> {
    let log_capture_mode = ServiceLogCaptureMode::from_name(log_capture_mode)
        .ok_or(GatewayManagementError::Unavailable)?;
    match (loopback_port, readiness_path, health_path) {
        (None, None, None) if log_capture_mode.is_disabled() => Ok(None),
        (Some(port), Some(readiness), Some(health)) => {
            let port = u16::try_from(port).map_err(|_| GatewayManagementError::Unavailable)?;
            let readiness = ServiceProbePath::parse(readiness)
                .map_err(|_| GatewayManagementError::Unavailable)?;
            let health =
                ServiceProbePath::parse(health).map_err(|_| GatewayManagementError::Unavailable)?;
            GatewayServiceConfig::new(port, readiness, health)
                .map(|service| service.with_log_capture_mode(log_capture_mode))
                .map(Some)
                .map_err(|_| GatewayManagementError::Unavailable)
        }
        _ => Err(GatewayManagementError::Unavailable),
    }
}
#[derive(sqlx::FromRow)]
struct GatewayRouteRow {
    id: Uuid,
    path: String,
    methods: Vec<String>,
    enabled: bool,
}
impl From<GatewayRouteRow> for GatewayManagementRoute {
    fn from(row: GatewayRouteRow) -> Self {
        Self {
            id: row.id,
            path: row.path,
            methods: row.methods,
            enabled: row.enabled,
        }
    }
}
#[derive(sqlx::FromRow)]
struct GatewayIngressRow {
    id: Uuid,
    gateway_revision_id: Uuid,
    gateway_route_id: Uuid,
    outcome: String,
    accepted_at: OffsetDateTime,
    completed_at: Option<OffsetDateTime>,
}
impl From<GatewayIngressRow> for GatewayIngressSummary {
    fn from(row: GatewayIngressRow) -> Self {
        Self {
            id: row.id,
            gateway_revision_id: row.gateway_revision_id,
            gateway_route_id: row.gateway_route_id,
            outcome: row.outcome,
            accepted_at: row.accepted_at,
            completed_at: row.completed_at,
        }
    }
}
#[derive(sqlx::FromRow)]
struct GatewayMailboxBindingRow {
    id: Uuid,
    gateway_revision_id: Uuid,
    mailbox_id: Uuid,
    slot_key: String,
    producer_id: String,
    grant_id: Uuid,
    grant_status: String,
    created_at: OffsetDateTime,
    granted_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}
impl From<GatewayMailboxBindingRow> for GatewayMailboxBindingSummary {
    fn from(row: GatewayMailboxBindingRow) -> Self {
        Self {
            id: row.id,
            gateway_revision_id: row.gateway_revision_id,
            mailbox_id: row.mailbox_id,
            slot_key: row.slot_key,
            producer_id: row.producer_id,
            grant_id: row.grant_id,
            grant_status: row.grant_status,
            created_at: row.created_at,
            granted_at: row.granted_at,
            revoked_at: row.revoked_at,
        }
    }
}
#[derive(sqlx::FromRow)]
struct GatewayMailboxBindingTargetRow {
    gateway_revision_id: Uuid,
    instance_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct GatewayMailboxBindingCommandRow {
    operation: String,
    gateway_revision_id: Uuid,
    slot_key: Option<String>,
    mailbox_id: Option<Uuid>,
    producer_id: Option<String>,
    target_binding_id: Option<Uuid>,
    payload_hash: Vec<u8>,
    actor_id: Uuid,
    result_binding_id: Option<Uuid>,
}

async fn load_mailbox_binding(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: Uuid,
) -> Result<Option<GatewayMailboxBindingRow>, sqlx::Error> {
    sqlx::query_as::<_, GatewayMailboxBindingRow>(
        "SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id,
                binding.slot_key, binding.producer_id, binding_grant.id AS grant_id,
                binding_grant.status AS grant_status, binding.created_at,
                binding_grant.granted_at, binding_grant.revoked_at
         FROM gateway_mailbox_bindings binding
         JOIN gateway_mailbox_binding_grants binding_grant
           ON binding_grant.binding_id = binding.id
         WHERE binding.id = $1",
    )
    .bind(binding_id)
    .fetch_optional(&mut **tx)
    .await
}

fn binding_command_key(identity: &AuthenticatedIdentity, operation: &str) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(operation, &[identity.idempotency_id.as_uuid().as_bytes()])
}

fn binding_payload_hash(
    operation: &str,
    gateway_revision_id: Uuid,
    slot_key: &str,
    mailbox_id: Option<Uuid>,
    producer_id: &str,
    target_binding_id: Option<Uuid>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(operation.as_bytes());
    digest.update([0]);
    digest.update(gateway_revision_id.as_bytes());
    digest.update([0]);
    digest.update(slot_key.as_bytes());
    digest.update([0]);
    if let Some(mailbox_id) = mailbox_id {
        digest.update(mailbox_id.as_bytes());
    }
    digest.update([0]);
    digest.update(producer_id.as_bytes());
    digest.update([0]);
    if let Some(target_binding_id) = target_binding_id {
        digest.update(target_binding_id.as_bytes());
    }
    digest.finalize().into()
}

#[derive(sqlx::FromRow)]
struct GatewayMailboxPublicationManagementRow {
    id: Uuid,
    invocation_id: Uuid,
    gateway_revision_id: Uuid,
    binding_id: Option<Uuid>,
    grant_id: Option<Uuid>,
    mailbox_id: Option<Uuid>,
    event_id: Option<Uuid>,
    slot_key: String,
    outcome: String,
    accepted_at: OffsetDateTime,
    settled_at: OffsetDateTime,
    authorization_snapshot_id: Option<Uuid>,
    snapshot_binding_ordinal: Option<i32>,
    delivery_disposition: Option<String>,
    delivery_attempt_count: Option<i32>,
    delivery_terminal_at: Option<OffsetDateTime>,
    delivery_attempt_id: Option<Uuid>,
    run_id: Option<Uuid>,
    run_state: Option<String>,
    run_outcome: Option<String>,
}
impl From<GatewayMailboxPublicationManagementRow> for GatewayMailboxPublicationSummary {
    fn from(row: GatewayMailboxPublicationManagementRow) -> Self {
        Self {
            id: row.id,
            invocation_id: row.invocation_id,
            gateway_revision_id: row.gateway_revision_id,
            binding_id: row.binding_id,
            grant_id: row.grant_id,
            mailbox_id: row.mailbox_id,
            event_id: row.event_id,
            slot_key: row.slot_key,
            outcome: row.outcome,
            accepted_at: row.accepted_at,
            settled_at: row.settled_at,
            authorization_snapshot_id: row.authorization_snapshot_id,
            snapshot_binding_ordinal: row.snapshot_binding_ordinal,
            delivery_disposition: row.delivery_disposition,
            delivery_attempt_count: row.delivery_attempt_count,
            delivery_terminal_at: row.delivery_terminal_at,
            delivery_attempt_id: row.delivery_attempt_id,
            run_id: row.run_id,
            run_state: row.run_state,
            run_outcome: row.run_outcome,
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

    /// Resolves a published release after checking project management
    /// authority. The returned commit is the immutable source coordinate the
    /// application layer must use to retrieve the repository gateway manifest.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, publication-state, or persistence
    /// failure. Draft and revoked releases are intentionally indistinguishable
    /// from unavailable release targets.
    pub async fn published_release(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: ReleaseId,
    ) -> Result<PublishedGatewayRelease, GatewayInstallError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = sqlx::query_as::<_, PublishedGatewayReleaseRow>(
            "SELECT release.id, repositories.project_id, release.repository_id,
                    release.source_commit, release.state
             FROM releases AS release
             JOIN repositories ON repositories.id = release.repository_id
             WHERE release.id = $1",
        )
        .bind(release_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayInstallError::Unavailable)?;
        self.require_manage(&mut tx, identity, ProjectId::from_uuid(target.project_id))
            .await?;
        if target.state != "published" {
            return Err(GatewayInstallError::Unavailable);
        }
        tx.commit().await?;
        Ok(PublishedGatewayRelease {
            release_id: ReleaseId::from_uuid(target.id),
            project_id: ProjectId::from_uuid(target.project_id),
            repository_id: RepositoryId::from_uuid(target.repository_id),
            source_commit: target.source_commit,
        })
    }

    /// Parses, authorizes, and installs the exact source manifest.
    ///
    /// Reinstalling an identical declaration selects its existing immutable
    /// revision. Changed source creates a fresh revision and only then makes
    /// it active, so a failure cannot leave a partially installed gateway.
    ///
    /// # Errors
    ///
    /// Returns a safe manifest, authorization, boundary, or persistence
    /// failure without exposing provider configuration or listener state.
    pub async fn install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallGatewayManifest,
    ) -> Result<InstallGatewayManifestResult, GatewayInstallError> {
        let config = parse_manifest(&command.manifest)?;
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        // Gateway rows are the receipt-producing aggregate. Mark this
        // mutation so an idempotent reinstall that keeps the same revision
        // still records one scoped product event for its fresh command key.
        sqlx::query("SET LOCAL hephaestus.gateway_install = 'true'")
            .execute(&mut *tx)
            .await?;
        self.require_manage(&mut tx, identity, command.project_id)
            .await?;
        require_repository_boundary(&mut tx, &command).await?;
        let command_key = installation_command_key(identity);
        if let Some(previous) =
            claim_installation_command(&mut tx, command_key, identity, &command).await?
        {
            tx.commit().await?;
            return Ok(InstallGatewayManifestResult { gateways: previous });
        }

        // Application role performs all authorization and command-ledger
        // writes. Immutable gateway materialization is a separate trusted
        // worker transition with the narrow grants declared by the migration.
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await?;

        let mut installed = Vec::with_capacity(config.gateways.len());
        for configured in config.gateways {
            let declaration = configured
                .to_declaration()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            let declaration_hash = declaration
                .validate()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            // A gateway is always released code, never a floating repository
            // command.  Resolve the symbolic manifest key before writing the
            // immutable revision so later dispatch cannot silently select a
            // different agent in the same release.
            let release_agent = resolve_release_agent(&mut tx, &command, &declaration).await?;
            let normalized_hash =
                installation_hash(declaration_hash, command.release_id, release_agent.id);
            installed.push(
                install_declaration(
                    &mut tx,
                    identity,
                    command.project_id,
                    command.repository_id,
                    command.release_id,
                    release_agent,
                    declaration,
                    normalized_hash,
                )
                .await?,
            );
        }
        for (ordinal, gateway) in installed.iter().enumerate() {
            sqlx::query(
                "INSERT INTO gateway_install_command_results
                    (command_key, ordinal, gateway_id, revision_id)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(command_key.as_bytes().as_slice())
            .bind(i32::try_from(ordinal + 1).map_err(|_| GatewayInstallError::Unavailable)?)
            .bind(gateway.gateway_id.as_uuid())
            .bind(gateway.revision_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(InstallGatewayManifestResult {
            gateways: installed,
        })
    }

    async fn require_manage(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        project_id: ProjectId,
    ) -> Result<(), GatewayInstallError> {
        let object = ObjectRef::new(ObjectType::Project, project_id.as_uuid());
        let decision = self
            .authorizer
            .check(
                tx,
                Subject::User(identity.user_id),
                Permission::CanManage,
                object,
            )
            .await
            .map_err(|_| GatewayInstallError::Unavailable)?;
        audit_decision(
            tx,
            identity.user_id,
            Permission::CanManage,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Preserve denied authorization evidence even though the command
            // transaction rolls back its in-transaction audit row.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                Permission::CanManage,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(GatewayInstallError::AuthorizationDenied)
        }
    }
}

/// Exact released agent selected by a repository gateway declaration.
#[derive(Debug, Clone)]
struct ReleaseAgentBinding {
    id: Uuid,
    key: String,
}

async fn resolve_release_agent(
    tx: &mut Transaction<'_, Postgres>,
    command: &InstallGatewayManifest,
    declaration: &GatewayDeclaration,
) -> Result<ReleaseAgentBinding, GatewayInstallError> {
    let release_id = command.release_id.ok_or(GatewayInstallError::Unavailable)?;
    sqlx::query_as::<_, ReleaseAgentBindingRow>(
        "SELECT agent.id, agent.agent_key AS key
         FROM release_agents AS agent
         JOIN releases AS release ON release.id = agent.release_id
         WHERE agent.release_id = $1
           AND agent.agent_key = $2
           AND release.repository_id = $3
           AND release.state = 'published'",
    )
    .bind(release_id.as_uuid())
    .bind(&declaration.agent_name)
    .bind(command.repository_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| ReleaseAgentBinding {
        id: row.id,
        key: row.key,
    })
    .ok_or(GatewayInstallError::Unavailable)
}

#[derive(sqlx::FromRow)]
struct PublishedGatewayReleaseRow {
    id: Uuid,
    project_id: Uuid,
    repository_id: Uuid,
    source_commit: String,
    state: String,
}

#[derive(sqlx::FromRow)]
struct InstallationCommandRow {
    operation: String,
    project_id: Uuid,
    repository_id: Uuid,
    release_id: Uuid,
    actor_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct InstallationCommandResultRow {
    gateway_id: Uuid,
    revision_id: Uuid,
}

fn installation_command_key(identity: &AuthenticatedIdentity) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(
        "install_release_gateways",
        &[identity.idempotency_id.as_uuid().as_bytes()],
    )
}

async fn claim_installation_command(
    tx: &mut Transaction<'_, Postgres>,
    key: ReleaseCommandKey,
    identity: &AuthenticatedIdentity,
    command: &InstallGatewayManifest,
) -> Result<Option<Vec<InstalledGateway>>, GatewayInstallError> {
    let inserted = sqlx::query(
        "INSERT INTO gateway_install_commands
             (command_key, operation, project_id, repository_id, release_id,
              actor_id, request_id)
         VALUES ($1, 'install_release_gateways', $2, $3, $4, $5, $6)
         ON CONFLICT (command_key) DO NOTHING",
    )
    .bind(key.as_bytes().as_slice())
    .bind(command.project_id.as_uuid())
    .bind(command.repository_id.as_uuid())
    .bind(
        command
            .release_id
            .ok_or(GatewayInstallError::Unavailable)?
            .as_uuid(),
    )
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 1 {
        return Ok(None);
    }
    let stored = sqlx::query_as::<_, InstallationCommandRow>(
        "SELECT operation, project_id, repository_id, release_id, actor_id
         FROM gateway_install_commands WHERE command_key = $1",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(GatewayInstallError::Unavailable)?;
    let release_id = command.release_id.ok_or(GatewayInstallError::Unavailable)?;
    if stored.operation != "install_release_gateways"
        || stored.project_id != command.project_id.as_uuid()
        || stored.repository_id != command.repository_id.as_uuid()
        || stored.release_id != release_id.as_uuid()
        || stored.actor_id != identity.user_id.as_uuid()
    {
        return Err(GatewayInstallError::Conflict);
    }
    let results = sqlx::query_as::<_, InstallationCommandResultRow>(
        "SELECT gateway_id, revision_id
         FROM gateway_install_command_results
         WHERE command_key = $1 ORDER BY ordinal",
    )
    .bind(key.as_bytes().as_slice())
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| InstalledGateway {
        gateway_id: GatewayId::from_uuid(row.gateway_id),
        revision_id: GatewayRevisionId::from_uuid(row.revision_id),
    })
    .collect();
    Ok(Some(results))
}

#[derive(sqlx::FromRow)]
struct ReleaseAgentBindingRow {
    id: Uuid,
    key: String,
}

fn parse_manifest(source: &[u8]) -> Result<RepositoryGatewaysConfig, GatewayInstallError> {
    let parsed = parse_repository_gateways(source);
    let config = parsed.config.ok_or(GatewayInstallError::InvalidManifest {
        diagnostics: parsed.diagnostics.clone(),
    })?;
    if config.gateways.is_empty() {
        return Err(GatewayInstallError::InvalidManifest {
            diagnostics: parsed.diagnostics,
        });
    }
    Ok(config)
}

async fn require_repository_boundary(
    tx: &mut Transaction<'_, Postgres>,
    command: &InstallGatewayManifest,
) -> Result<(), GatewayInstallError> {
    let repository_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM repositories WHERE id = $1 AND project_id = $2)",
    )
    .bind(command.repository_id.as_uuid())
    .bind(command.project_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    if !repository_exists {
        return Err(GatewayInstallError::Unavailable);
    }
    if let Some(release_id) = command.release_id {
        let release_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM releases
                 JOIN repositories ON repositories.id = releases.repository_id
                 WHERE releases.id = $1
                   AND releases.repository_id = $2
                   AND repositories.project_id = $3
                   AND releases.state = 'published'
             )",
        )
        .bind(release_id.as_uuid())
        .bind(command.repository_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .fetch_one(&mut **tx)
        .await?;
        if !release_exists {
            return Err(GatewayInstallError::Unavailable);
        }
    }
    Ok(())
}

// Keep the declaration, route, and authority materialization in one atomic
// transaction despite the bounded number of immutable row fields.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn install_declaration(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    project_id: ProjectId,
    repository_id: RepositoryId,
    release_id: Option<ReleaseId>,
    release_agent: ReleaseAgentBinding,
    declaration: GatewayDeclaration,
    normalized_hash: [u8; 32],
) -> Result<InstalledGateway, GatewayInstallError> {
    let gateway_id = GatewayId::from_uuid(
        sqlx::query_scalar(
            "INSERT INTO gateways (id, project_id, repository_id, name, lifecycle, created_by)
             VALUES ($1, $2, $3, $4, 'enabled', $5)
             ON CONFLICT (repository_id, name) DO UPDATE SET updated_at = now()
               WHERE gateways.lifecycle <> 'removed'
             RETURNING id",
        )
        .bind(GatewayId::new().as_uuid())
        .bind(project_id.as_uuid())
        .bind(repository_id.as_uuid())
        .bind(declaration.name.as_str())
        .bind(identity.user_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(GatewayInstallError::Unavailable)?,
    );
    let revision_id = GatewayRevisionId::new();
    let inserted_revision: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters, secret_slots,
             mailbox_slots, service_loopback_port, service_readiness_path,
             service_health_path, service_log_capture_mode, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (gateway_id, normalized_hash) DO NOTHING
         RETURNING id",
    )
    .bind(revision_id.as_uuid())
    .bind(gateway_id.as_uuid())
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .bind(release_id.map(ReleaseId::as_uuid))
    .bind(release_agent.id)
    .bind(release_agent.key)
    .bind(&declaration.handler_contract)
    .bind(exposure_name(declaration.exposure))
    .bind(&declaration.parameters)
    .bind(&declaration.secret_slots)
    .bind(
        declaration
            .mailbox_publication_slots
            .iter()
            .map(|slot| slot.key.as_str())
            .collect::<Vec<_>>(),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| i32::from(service.loopback_port)),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| service.readiness_path.as_str()),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map(|service| service.health_path.as_str()),
    )
    .bind(
        declaration
            .service
            .as_ref()
            .map_or(ServiceLogCaptureMode::Disabled, |service| {
                service.log_capture_mode
            })
            .as_str(),
    )
    .bind(normalized_hash.as_slice())
    .bind(identity.user_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let revision_id =
        GatewayRevisionId::from_uuid(match inserted_revision {
            Some(id) => {
                for route in &declaration.routes {
                    sqlx::query(
                        "INSERT INTO gateway_routes
                        (id, gateway_revision_id, gateway_id, project_id, path, methods)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    )
                    .bind(Uuid::new_v4())
                    .bind(id)
                    .bind(gateway_id.as_uuid())
                    .bind(project_id.as_uuid())
                    .bind(route.path.as_str())
                    .bind(
                        route
                            .methods
                            .iter()
                            .map(|method| method_name(*method))
                            .collect::<Vec<_>>(),
                    )
                    .execute(&mut **tx)
                    .await?;
                }
                id
            }
            None => sqlx::query_scalar(
                "SELECT id FROM gateway_revisions WHERE gateway_id = $1 AND normalized_hash = $2",
            )
            .bind(gateway_id.as_uuid())
            .bind(normalized_hash.as_slice())
            .fetch_one(&mut **tx)
            .await?,
        });
    if declaration.handler_contract == "http.service.v1" {
        // A service declaration is durable desired state. It becomes
        // publicly serving only after a later readiness-gated activation.
        sqlx::query(
            "UPDATE gateways SET desired_service_revision_id = $2, updated_at = now()
             WHERE id = $1",
        )
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    } else {
        // Stateless gateways retain their existing atomic immediate-activation
        // behavior and supersede any pending service declaration.
        sqlx::query(
            "UPDATE gateways SET active_revision_id = $2,
                    desired_service_revision_id = NULL, updated_at = now()
             WHERE id = $1",
        )
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
    }
    Ok(InstalledGateway {
        gateway_id,
        revision_id,
    })
}

fn installation_hash(
    declaration_hash: [u8; 32],
    release_id: Option<ReleaseId>,
    release_agent_id: Uuid,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus-gateway-install-v1\0");
    digest.update(declaration_hash);
    if let Some(release_id) = release_id {
        digest.update(release_id.as_uuid().as_bytes());
    }
    digest.update(release_agent_id.as_bytes());
    digest.finalize().into()
}

const fn exposure_name(exposure: Exposure) -> &'static str {
    match exposure {
        Exposure::Public => "public",
        Exposure::HephAuthenticated => "heph_authenticated",
    }
}

const fn method_name(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
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
