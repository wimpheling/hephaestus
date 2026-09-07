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
use gateway_domain::{Exposure, GatewayDeclaration, GatewayId, GatewayRevisionId, HttpMethod};
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayInvocationOutcome,
    GatewayInvocationRecorder, GatewayLimits, GatewayMailboxPublisher, GatewayReleaseResolver,
    GatewayRouteBinding, GatewayRouteResolver,
};
use http::Method;
use identity_domain::AuthenticatedIdentity;
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEventId, SelectedHeaderName, SelectedHeaderValue,
    TraceContext,
};
use release_domain::{
    ParameterDeclaration, ParameterDocument, ParameterName, ParameterValue, ReleaseCommandKey,
    ReleaseId,
};
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeHandoffStore,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, PrivateMailboxPublication, RootFilesystem,
    RuntimeAuthorityBootstrap, VmId, VmMount, VmResources, VmSpec,
};

/// Host-only request to publish one generic event through an exact gateway
/// mailbox slot.  The caller never chooses the mailbox or producer identity.
#[derive(Debug, Clone)]
pub struct GatewayMailboxPublicationRequest {
    /// Session authenticated by the gateway runtime handoff.
    pub runtime_session_id: Uuid,
    /// Invocation currently executing that session.
    pub invocation_id: Uuid,
    /// Declared immutable slot selected by the guest protocol.
    pub slot_key: String,
    /// Application-selected stable key within the bound producer scope.
    pub deduplication_key: DeduplicationKey,
    /// Bounded, value-bearing envelope. It is never written to gateway audit.
    pub envelope: MailboxEnvelope,
    /// Opaque bytes corresponding exactly to the envelope body reference.
    pub encoded_body: Vec<u8>,
    /// Decoded length; this MVP accepts only identity encoding.
    pub decoded_length: u32,
}

/// Redacted durable result of a gateway mailbox publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayMailboxPublicationResult {
    /// One new mailbox event and its transactional wake command were accepted.
    Accepted {
        /// Immutable accepted mailbox event.
        event_id: MailboxEventId,
    },
    /// The same invocation/slot/key was already accepted.
    Duplicate {
        /// Original immutable mailbox event.
        event_id: MailboxEventId,
    },
    /// Live authority was unavailable; details deliberately remain redacted.
    Denied,
}

/// Worker-side authority for the gateway-to-mailbox bridge.
#[derive(Clone)]
pub struct PostgresGatewayMailboxPublisher {
    pool: PgPool,
}

impl PostgresGatewayMailboxPublisher {
    /// Creates the publisher over the control-plane authority pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Rechecks the live invocation/session/revision/binding/grant/mailbox
    /// chain, then accepts through the normal mailbox event and outbox path.
    ///
    /// # Errors
    ///
    /// Returns an unavailable error for invalid payload evidence or database
    /// failures. Authority denial is persisted and returned without detail.
    #[allow(clippy::too_many_lines)]
    pub async fn publish(
        &self,
        request: GatewayMailboxPublicationRequest,
    ) -> Result<GatewayMailboxPublicationResult, GatewayEdgeError> {
        validate_gateway_mailbox_payload(&request)?;
        let headers = serde_json::to_value(&request.envelope.headers)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        // Serializing on the invocation makes a repeated guest frame observe
        // its earlier publication before it can allocate another body row.
        // Updates close execution, not durable ingress. The dispatcher binds
        // queued work only after the gate reopens; recovery states fail closed.
        let invocation: Option<GatewayMailboxAuthorityRow> = sqlx::query_as(
            "SELECT invocation.gateway_revision_id AS revision, binding.id AS binding,
                    binding_grant.id AS grant_id, binding.mailbox_id AS mailbox, binding.producer_id AS producer
             FROM gateway_invocations AS invocation
             JOIN gateway_runtime_authority_sessions AS session
               ON session.id = $1 AND session.invocation_id = invocation.id
             JOIN gateway_authorization_snapshot_bindings AS snapshot_binding
               ON snapshot_binding.snapshot_id = session.snapshot_id
             JOIN gateway_mailbox_bindings AS binding
               ON binding.gateway_revision_id = invocation.gateway_revision_id
              AND binding.slot_key = $3
              AND snapshot_binding.binding_id = binding.id
             JOIN gateway_mailbox_binding_grants AS binding_grant
               ON binding_grant.binding_id = binding.id
              AND binding_grant.status = 'active'
              AND snapshot_binding.grant_id = binding_grant.id
             JOIN mailboxes AS mailbox ON mailbox.id = binding.mailbox_id
             JOIN agent_instances AS instance ON instance.id = mailbox.instance_id
             JOIN gateways AS gateway ON gateway.id = invocation.gateway_id
             WHERE invocation.id = $2
               AND invocation.outcome = 'accepted'
               AND session.status = 'active' AND session.expires_at > now()
               AND gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = invocation.gateway_revision_id
               AND mailbox.state = 'active'
               AND instance.state IN ('active', 'update_rejected', 'update_draining', 'updating')
             FOR UPDATE OF invocation, session",
        )
        .bind(request.runtime_session_id)
        .bind(request.invocation_id)
        .bind(&request.slot_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| {
            tracing::warn!(%error, invocation_id = %request.invocation_id, "gateway mailbox authority query failed");
            GatewayEdgeError::Unavailable
        })?;
        if let Some(existing) = sqlx::query_as::<_, GatewayMailboxPublicationRow>(
            "SELECT outcome, event_id FROM gateway_mailbox_publications
             WHERE invocation_id = $1 AND slot_key = $2 AND deduplication_key = $3",
        )
        .bind(request.invocation_id)
        .bind(&request.slot_key)
        .bind(request.deduplication_key.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        {
            tx.commit()
                .await
                .map_err(|_| GatewayEdgeError::Unavailable)?;
            return repeated_publication_result(&existing);
        }
        let Some(authority) = invocation else {
            // A valid runtime session is still retained as the denial's
            // correlation. A forged/missing session cannot create evidence.
            sqlx::query(
                "INSERT INTO gateway_mailbox_publications
                   (id, invocation_id, runtime_session_id, gateway_revision_id, slot_key,
                    deduplication_key, outcome, denial_code)
                 SELECT gen_random_uuid(), invocation.id, $1, invocation.gateway_revision_id,
                        $3, $4, 'denied', 'authority_unavailable'
                 FROM gateway_invocations AS invocation
                 JOIN gateway_runtime_authority_sessions AS session
                   ON session.id = $1 AND session.invocation_id = invocation.id
                 WHERE invocation.id = $2
                 ON CONFLICT (invocation_id, slot_key, deduplication_key) DO NOTHING",
            )
            .bind(request.runtime_session_id)
            .bind(request.invocation_id)
            .bind(&request.slot_key)
            .bind(request.deduplication_key.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
            tx.commit()
                .await
                .map_err(|_| GatewayEdgeError::Unavailable)?;
            return Ok(GatewayMailboxPublicationResult::Denied);
        };
        let body = &request.envelope.content.body;
        let event_id = Uuid::new_v4();
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO mailbox_payloads
               (id, mailbox_id, project_id, encoded_body, encoded_length, decoded_length,
                integrity_hash, content_type, content_encoding)
             SELECT $1, binding.mailbox_id, binding.project_id, $2, $3, $4, $5, $6, $7
             FROM gateway_mailbox_bindings AS binding WHERE binding.id = $8
             RETURNING id",
        )
        .bind(body.id.as_uuid())
        .bind(&request.encoded_body)
        .bind(i32::try_from(request.encoded_body.len()).map_err(|_| GatewayEdgeError::Unavailable)?)
        .bind(i32::try_from(request.decoded_length).map_err(|_| GatewayEdgeError::Unavailable)?)
        .bind(body.integrity_hash.as_slice())
        .bind(&request.envelope.content.content_type)
        .bind(&request.envelope.content.content_encoding)
        .bind(authority.binding)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let event = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO mailbox_events
               (id, mailbox_id, project_id, instance_id, body_id, producer_kind, producer_id,
                deduplication_scope, deduplication_key, method, route, selected_headers,
                content_type, received_at, trace_context)
             SELECT $1, binding.mailbox_id, binding.project_id, mailbox.instance_id, $2,
                    'gateway', binding.producer_id, binding.producer_id, $3, $4, $5, $6,
                    $7, $8, $9
             FROM gateway_mailbox_bindings AS binding JOIN mailboxes AS mailbox ON mailbox.id = binding.mailbox_id
             WHERE binding.id = $10
             ON CONFLICT (mailbox_id, deduplication_scope, deduplication_key) DO NOTHING
             RETURNING id",
        ).bind(event_id).bind(inserted).bind(request.deduplication_key.as_str())
         .bind(request.envelope.method.as_str()).bind(request.envelope.route.as_str()).bind(headers)
         .bind(&request.envelope.content.content_type).bind(request.envelope.received_at)
         .bind(request.envelope.trace_context.as_ref().map(mailbox_domain::TraceContext::as_str))
         .bind(authority.binding).fetch_optional(&mut *tx)
         .await.map_err(|_| GatewayEdgeError::Unavailable)?;
        let (event_id, outcome) = if let Some(id) = event {
            (id, "accepted")
        } else {
            // The candidate payload must not survive a deduplication collision.
            tx.rollback()
                .await
                .map_err(|_| GatewayEdgeError::Unavailable)?;
            return self.record_gateway_mailbox_duplicate(request).await;
        };
        sqlx::query(
            "INSERT INTO gateway_mailbox_publications
               (id, invocation_id, runtime_session_id, gateway_revision_id, binding_id, grant_id,
                mailbox_id, producer_id, slot_key, deduplication_key, event_id, outcome)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(request.invocation_id)
        .bind(request.runtime_session_id)
        .bind(authority.revision)
        .bind(authority.binding)
        .bind(authority.grant_id)
        .bind(authority.mailbox)
        .bind(authority.producer)
        .bind(&request.slot_key)
        .bind(request.deduplication_key.as_str())
        .bind(event_id)
        .bind(outcome)
        .execute(&mut *tx)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        tx.commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(GatewayMailboxPublicationResult::Accepted {
            event_id: MailboxEventId::from_uuid(event_id),
        })
    }

    async fn record_gateway_mailbox_duplicate(
        &self,
        request: GatewayMailboxPublicationRequest,
    ) -> Result<GatewayMailboxPublicationResult, GatewayEdgeError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let row: Option<GatewayMailboxAuthorityRow> = sqlx::query_as(
            "SELECT invocation.gateway_revision_id AS revision, binding.id AS binding,
                    binding_grant.id AS grant_id, binding.mailbox_id AS mailbox, binding.producer_id AS producer
             FROM gateway_invocations invocation JOIN gateway_runtime_authority_sessions session
               ON session.id = $1 AND session.invocation_id = invocation.id
             JOIN gateway_authorization_snapshot_bindings snapshot_binding
               ON snapshot_binding.snapshot_id = session.snapshot_id
             JOIN gateway_mailbox_bindings binding
               ON binding.gateway_revision_id = invocation.gateway_revision_id
              AND binding.slot_key = $3
              AND snapshot_binding.binding_id = binding.id
             JOIN gateway_mailbox_binding_grants binding_grant
               ON binding_grant.binding_id = binding.id
              AND binding_grant.status = 'active'
              AND snapshot_binding.grant_id = binding_grant.id
             JOIN mailboxes mailbox ON mailbox.id = binding.mailbox_id
             JOIN agent_instances instance ON instance.id = mailbox.instance_id
             JOIN gateways gateway ON gateway.id = invocation.gateway_id
             WHERE invocation.id = $2 AND invocation.outcome = 'accepted'
               AND session.status = 'active' AND session.expires_at > now()
               AND mailbox.state = 'active'
               AND instance.state IN ('active', 'update_rejected', 'update_draining', 'updating')
               AND gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = invocation.gateway_revision_id
             FOR UPDATE OF invocation, session",
        ).bind(request.runtime_session_id).bind(request.invocation_id).bind(&request.slot_key)
         .fetch_optional(&mut *tx).await.map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some(authority) = row else {
            return Ok(GatewayMailboxPublicationResult::Denied);
        };
        let event_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM mailbox_events WHERE mailbox_id = $1 AND deduplication_scope = $2 AND deduplication_key = $3",
        ).bind(authority.mailbox).bind(&authority.producer).bind(request.deduplication_key.as_str())
         .fetch_one(&mut *tx).await.map_err(|_| GatewayEdgeError::Unavailable)?;
        sqlx::query(
            "INSERT INTO gateway_mailbox_publications
               (id, invocation_id, runtime_session_id, gateway_revision_id, binding_id, grant_id, mailbox_id,
                producer_id, slot_key, deduplication_key, event_id, outcome)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'duplicate')
             ON CONFLICT (invocation_id, slot_key, deduplication_key) DO NOTHING",
        ).bind(request.invocation_id).bind(request.runtime_session_id).bind(authority.revision)
         .bind(authority.binding).bind(authority.grant_id).bind(authority.mailbox)
         .bind(authority.producer).bind(&request.slot_key).bind(request.deduplication_key.as_str())
         .bind(event_id).execute(&mut *tx).await.map_err(|_| GatewayEdgeError::Unavailable)?;
        tx.commit()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        Ok(GatewayMailboxPublicationResult::Duplicate {
            event_id: MailboxEventId::from_uuid(event_id),
        })
    }
}

#[async_trait]
impl GatewayMailboxPublisher for PostgresGatewayMailboxPublisher {
    async fn publish(
        &self,
        invocation_id: Uuid,
        publication: PrivateMailboxPublication,
    ) -> Result<(), GatewayEdgeError> {
        let encoded_body = publication.body.to_vec();
        let byte_length =
            u32::try_from(encoded_body.len()).map_err(|_| GatewayEdgeError::Unavailable)?;
        let integrity_hash: [u8; 32] = Sha256::digest(&encoded_body).into();
        let body = BodyReference::new(BodyReferenceId::new(), byte_length, integrity_hash)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        let content = ContentMetadata::new(
            body,
            publication.content_type,
            Some(String::from("identity")),
        )
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let headers = publication
            .headers
            .into_iter()
            .map(|(name, value)| {
                Ok((
                    SelectedHeaderName::parse(name).map_err(|_| GatewayEdgeError::Unavailable)?,
                    SelectedHeaderValue::parse(value).map_err(|_| GatewayEdgeError::Unavailable)?,
                ))
            })
            .collect::<Result<_, GatewayEdgeError>>()?;
        let envelope = MailboxEnvelope::new(
            EnvelopeMethod::parse(publication.method).map_err(|_| GatewayEdgeError::Unavailable)?,
            EnvelopeRoute::parse(publication.route).map_err(|_| GatewayEdgeError::Unavailable)?,
            headers,
            content,
            OffsetDateTime::now_utc(),
            publication
                .trace_context
                .map(TraceContext::parse)
                .transpose()
                .map_err(|_| GatewayEdgeError::Unavailable)?,
        )
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        // Gateway runtime session identity is deterministically the accepted
        // invocation identity. The publisher rechecks that exact session only
        // after entering its worker-role transaction; guests never provide it.
        let runtime_session_id = invocation_id;
        let result = self
            .publish(GatewayMailboxPublicationRequest {
                runtime_session_id,
                invocation_id,
                slot_key: publication.slot,
                deduplication_key: DeduplicationKey::parse(publication.deduplication_key)
                    .map_err(|_| GatewayEdgeError::Unavailable)?,
                envelope,
                encoded_body,
                decoded_length: byte_length,
            })
            .await?;
        match result {
            GatewayMailboxPublicationResult::Accepted { .. }
            | GatewayMailboxPublicationResult::Duplicate { .. } => Ok(()),
            GatewayMailboxPublicationResult::Denied => Err(GatewayEdgeError::HandlerUnavailable),
        }
    }
}

#[derive(sqlx::FromRow)]
struct GatewayMailboxAuthorityRow {
    revision: Uuid,
    binding: Uuid,
    grant_id: Uuid,
    mailbox: Uuid,
    producer: String,
}
#[derive(sqlx::FromRow)]
struct GatewayMailboxPublicationRow {
    outcome: String,
    event_id: Option<Uuid>,
}

/// The stored outcome records the original decision; a later observation of
/// that row is always an idempotent duplicate of the guest publication.
fn repeated_publication_result(
    row: &GatewayMailboxPublicationRow,
) -> Result<GatewayMailboxPublicationResult, GatewayEdgeError> {
    match (row.outcome.as_str(), row.event_id) {
        ("accepted" | "duplicate", Some(id)) => Ok(GatewayMailboxPublicationResult::Duplicate {
            event_id: MailboxEventId::from_uuid(id),
        }),
        ("denied", None) => Ok(GatewayMailboxPublicationResult::Denied),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}

fn validate_gateway_mailbox_payload(
    request: &GatewayMailboxPublicationRequest,
) -> Result<(), GatewayEdgeError> {
    let body = &request.envelope.content.body;
    if request.encoded_body.len()
        != usize::try_from(body.byte_length).map_err(|_| GatewayEdgeError::Unavailable)?
        || request.decoded_length != body.byte_length
        || Sha256::digest(&request.encoded_body).as_slice() != body.integrity_hash
        || request
            .envelope
            .content
            .content_encoding
            .as_deref()
            .is_some_and(|value| value != "identity")
    {
        return Err(GatewayEdgeError::Unavailable);
    }
    Ok(())
}

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

    async fn active_routes(&self) -> Result<Vec<GatewayRouteBinding>, GatewayEdgeError> {
        let rows = sqlx::query_as::<_, ActiveRouteRow>(
            "SELECT route.id AS route_id, route.gateway_revision_id, route.path, route.methods
             FROM gateway_routes AS route
             JOIN gateways AS gateway ON gateway.id = route.gateway_id
             WHERE gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = route.gateway_revision_id
               AND route.enabled
             ORDER BY route.path, route.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        rows.into_iter()
            .map(|row| active_route(row, self.limits))
            .collect()
    }
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
        let accepted = sqlx::query_as::<_, AcceptedInvocationRow>(
            "INSERT INTO gateway_invocations
                 (id, gateway_id, gateway_revision_id, gateway_route_id, project_id, request_id, outcome)
             SELECT $1, route.gateway_id, route.gateway_revision_id, route.id, route.project_id, $2, 'accepted'
             FROM gateway_routes AS route
             JOIN gateways AS gateway ON gateway.id = route.gateway_id
             WHERE route.id = $3
               AND route.gateway_revision_id = $4
               AND route.enabled
               AND gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = route.gateway_revision_id
             RETURNING gateway_id, gateway_revision_id",
        )
        .bind(invocation_id)
        .bind(request_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some(accepted) = accepted else {
            return Err(GatewayEdgeError::Unavailable);
        };
        if let Some(issuer) = &self.runtime_authority {
            let issued_at = OffsetDateTime::now_utc();
            let issued = issuer
                .issue_gateway(GatewayRuntimeSessionRequest {
                    invocation_id: capability_domain::GatewayInvocationId::from_uuid(invocation_id),
                    gateway_id: accepted.gateway_id,
                    gateway_revision_id: accepted.gateway_revision_id,
                    issued_at,
                    expires_at: issued_at
                        + time::Duration::try_from(self.session_ttl)
                            .map_err(|_| GatewayEdgeError::Unavailable)?,
                })
                .await;
            let Ok(issued) = issued else {
                if let Err(error) = issued {
                    tracing::warn!(%error, invocation_id = %invocation_id, "gateway runtime authority issuance failed");
                }
                let _: bool =
                    sqlx::query_scalar("SELECT gateway_invocation_complete($1, 'rejected')")
                        .bind(invocation_id)
                        .fetch_one(&self.pool)
                        .await
                        .map_err(|_| GatewayEdgeError::Unavailable)?;
                return Err(GatewayEdgeError::Unavailable);
            };
            // Leases are derived only after the exact gateway runtime session
            // exists. Their trigger rechecks invocation, revision, route,
            // import, selected version, and active lifecycle ceilings.
            sqlx::query(
                "INSERT INTO gateway_secret_leases
                     (id, runtime_session_id, invocation_id, binding_id,
                      secret_version_id, rule_id, status, expires_at)
                 SELECT gen_random_uuid(), $1, $2, binding.id,
                        binding.secret_version_id, rule.id, 'active', session.expires_at
                 FROM gateway_runtime_authority_sessions AS session
                 JOIN gateway_invocations AS invocation ON invocation.id = session.invocation_id
                 JOIN gateway_brokered_secret_rules AS rule
                   ON rule.gateway_revision_id = invocation.gateway_revision_id
                  AND rule.gateway_route_id = invocation.gateway_route_id
                 JOIN gateway_secret_bindings AS binding
                   ON binding.id = rule.binding_id
                  AND binding.gateway_revision_id = invocation.gateway_revision_id
                 WHERE session.id = $1 AND session.invocation_id = $2
                   AND session.status IN ('pending_handoff', 'active')
                   AND session.expires_at > now()
                   AND binding.status = 'active'
                 ON CONFLICT (invocation_id, rule_id) DO NOTHING",
            )
            .bind(issued.id.as_uuid())
            .bind(invocation_id)
            .execute(&self.pool)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        }
        Ok(invocation_id)
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

/// Production resolver for the exact released agent selected by a gateway revision.
///
/// It accepts only an already-recorded invocation ID; public request data cannot
/// influence image, command, resource, or bootstrap selection.
pub struct PostgresGatewayReleaseResolver {
    pool: PgPool,
    root_images: BTreeMap<String, RootFilesystem>,
    handoff: Arc<dyn RuntimeHandoffStore>,
    release_materializer: Option<Arc<dyn GatewayReleaseMaterializer>>,
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

impl PostgresGatewayReleaseResolver {
    /// Creates a resolver over operator-materialized immutable root filesystems
    /// and the same host-only handoff store used to issue gateway sessions.
    #[must_use]
    pub fn new(
        pool: PgPool,
        root_images: BTreeMap<String, RootFilesystem>,
        handoff: Arc<dyn RuntimeHandoffStore>,
    ) -> Self {
        Self {
            pool,
            root_images,
            handoff,
            release_materializer: None,
        }
    }

    /// Attaches the host-owned release-tree lifecycle required for a real VM
    /// launch. A resolver without this explicit boundary fails closed rather
    /// than executing a path supplied by the base image.
    #[must_use]
    pub fn with_release_materializer(
        mut self,
        release_materializer: Arc<dyn GatewayReleaseMaterializer>,
    ) -> Self {
        self.release_materializer = Some(release_materializer);
        self
    }

    /// Cleans up the invocation's materialized release tree after the VM has
    /// been destroyed.
    ///
    /// # Errors
    ///
    /// Returns an unavailable edge error when no configured materializer can
    /// safely remove the invocation's release tree.
    pub fn destroy_materialized_release(
        &self,
        invocation_id: Uuid,
    ) -> Result<(), GatewayEdgeError> {
        self.release_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?
            .destroy(invocation_id)
    }
}

#[async_trait]
impl GatewayReleaseResolver for PostgresGatewayReleaseResolver {
    // The complete authority query, artifact validation, and fail-closed VM
    // specification are deliberately adjacent for one auditable launch path.
    #[allow(clippy::too_many_lines)]
    async fn resolve_launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<VmSpec, GatewayEdgeError> {
        let row = sqlx::query_as::<_, GatewayLaunchRow>(
            "SELECT agent.runtime_contract, session.issuance_generation, revision.release_id, revision.parameters
             FROM gateway_invocations AS invocation
             JOIN gateway_revisions AS revision
               ON revision.id = invocation.gateway_revision_id
              AND revision.gateway_id = invocation.gateway_id
             JOIN release_agents AS agent
               ON agent.id = revision.release_agent_id
              AND agent.release_id = revision.release_id
              AND agent.agent_key = revision.release_agent_key
             JOIN releases AS release
               ON release.id = revision.release_id
              AND release.repository_id = revision.repository_id
             JOIN gateway_runtime_authority_sessions AS session
               ON session.invocation_id = invocation.id
              AND session.gateway_id = invocation.gateway_id
              AND session.gateway_revision_id = invocation.gateway_revision_id
             WHERE invocation.id = $1
               AND invocation.gateway_route_id = $2
               AND invocation.gateway_revision_id = $3
               AND invocation.outcome = 'accepted'
               AND revision.handler_contract = 'http.v1'
               AND release.state = 'published'
               AND session.status = 'pending_handoff'
               AND session.expires_at > now()",
        )
        .bind(invocation_id)
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let contract: GatewayRuntimeContract = serde_json::from_value(row.runtime_contract)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        // MVP-03 gateway VMs are stateless one-request handlers. They cannot
        // silently acquire a volume or general network listener merely because
        // the source agent also supports those modes elsewhere.
        if contract.requires_state
            || !matches!(contract.policy_ceiling.network, GatewayNetwork::Disabled)
        {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let root = self
            .root_images
            .get(&contract.image_reference)
            .cloned()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let materializer = self
            .release_materializer
            .as_ref()
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let artifacts = gateway_release_artifacts(&self.pool, row.release_id).await?;
        let mounts = materializer.prepare(invocation_id, &artifacts, &row.parameters)?;
        let generation = u64::try_from(row.issuance_generation)
            .ok()
            .and_then(|value| capability_domain::RuntimeCredentialGeneration::new(value).ok())
            .ok_or(GatewayEdgeError::Unavailable)?;
        let session_id = capability_domain::RuntimeSessionId::from_uuid(invocation_id);
        let credential = self
            .handoff
            .open(session_id, generation, OffsetDateTime::now_utc())
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        Ok(VmSpec {
            id: VmId(format!("gateway-{invocation_id}")),
            root,
            disks: Vec::new(),
            mounts,
            resources: VmResources {
                vcpus: contract.policy_ceiling.vcpus,
                memory_mib: contract.policy_ceiling.memory_mib,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: format!("/release/{}", contract.command),
                args: contract.arguments,
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from(format!(
                    "/release/{}",
                    contract.working_directory
                ))),
            },
            runtime_authority: Some(RuntimeAuthorityBootstrap::new(
                invocation_id,
                generation.get(),
                *credential.expose(),
            )),
            labels: BTreeMap::from([
                (String::from("hephaestus.kind"), String::from("gateway")),
                (
                    String::from("hephaestus.gateway.handler-contract"),
                    String::from("http.v1"),
                ),
                (
                    String::from("hephaestus.gateway-invocation"),
                    invocation_id.to_string(),
                ),
                (
                    String::from("hephaestus.gateway-route"),
                    route.route_id.to_string(),
                ),
                (
                    String::from("hephaestus.gateway-revision"),
                    route.gateway_revision_id.to_string(),
                ),
            ]),
        })
    }

    async fn acknowledge_runtime_authority(
        &self,
        invocation_id: Uuid,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), GatewayEdgeError> {
        let generation = i64::try_from(generation).map_err(|_| GatewayEdgeError::Unavailable)?;
        let acknowledged = sqlx::query_scalar::<_, bool>(
            "UPDATE gateway_runtime_authority_sessions
                SET status = 'active', acknowledged_at = now(), updated_at = now()
              WHERE id = $1
                AND invocation_id = $2
                AND issuance_generation = $3
                AND status = 'pending_handoff'
                AND issued_at <= now()
                AND expires_at > now()
              RETURNING TRUE",
        )
        .bind(session_id)
        .bind(invocation_id)
        .bind(generation)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        if acknowledged == Some(true) {
            Ok(())
        } else {
            Err(GatewayEdgeError::HandlerUnavailable)
        }
    }

    async fn cleanup_launch(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError> {
        self.destroy_materialized_release(invocation_id)
    }
}

#[derive(sqlx::FromRow)]
struct GatewayLaunchRow {
    runtime_contract: serde_json::Value,
    issuance_generation: i64,
    release_id: Uuid,
    parameters: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct GatewayReleaseArtifactRow {
    path: String,
    kind: String,
    mode: i32,
    content_hash: Vec<u8>,
    size_bytes: i64,
    storage_key: Uuid,
}

async fn gateway_release_artifacts(
    pool: &PgPool,
    release_id: Uuid,
) -> Result<Vec<GatewayReleaseArtifact>, GatewayEdgeError> {
    let rows = sqlx::query_as::<_, GatewayReleaseArtifactRow>(
        "SELECT path, kind, mode, content_hash, size_bytes, storage_key
           FROM release_artifacts
          WHERE release_id = $1
          ORDER BY path",
    )
    .bind(release_id)
    .fetch_all(pool)
    .await
    .map_err(|_| GatewayEdgeError::Unavailable)?;
    rows.into_iter().map(gateway_release_artifact).collect()
}

fn gateway_release_artifact(
    row: GatewayReleaseArtifactRow,
) -> Result<GatewayReleaseArtifact, GatewayEdgeError> {
    let content_hash: [u8; 32] = row
        .content_hash
        .try_into()
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let mode = u32::try_from(row.mode).map_err(|_| GatewayEdgeError::Unavailable)?;
    let size_bytes = u64::try_from(row.size_bytes).map_err(|_| GatewayEdgeError::Unavailable)?;
    let kind = match row.kind.as_str() {
        "executable" => GatewayReleaseArtifactKind::Executable,
        "file" => GatewayReleaseArtifactKind::File,
        "manifest" => GatewayReleaseArtifactKind::Manifest,
        _ => return Err(GatewayEdgeError::Unavailable),
    };
    Ok(GatewayReleaseArtifact {
        path: row.path,
        kind,
        mode,
        content_hash,
        size_bytes,
        storage_key: row.storage_key,
    })
}

#[derive(Deserialize)]
struct GatewayRuntimeContract {
    #[serde(alias = "executable")]
    command: String,
    arguments: Vec<String>,
    working_directory: String,
    image_reference: String,
    requires_state: bool,
    policy_ceiling: GatewayPolicyCeiling,
}

#[derive(Deserialize)]
struct GatewayPolicyCeiling {
    vcpus: u8,
    memory_mib: u32,
    network: GatewayNetwork,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GatewayNetwork {
    Disabled,
    BrokerOnly,
    Egress,
}

#[derive(sqlx::FromRow)]
struct ActiveRouteRow {
    route_id: Uuid,
    gateway_revision_id: Uuid,
    path: String,
    methods: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct AcceptedInvocationRow {
    gateway_id: Uuid,
    gateway_revision_id: Uuid,
}

fn active_route(
    row: ActiveRouteRow,
    limits: GatewayLimits,
) -> Result<GatewayRouteBinding, GatewayEdgeError> {
    let methods = row
        .methods
        .into_iter()
        .map(|method| persisted_method(&method))
        .collect::<Result<_, _>>()?;
    let path_prefix = row
        .path
        .strip_prefix('/')
        .ok_or(GatewayEdgeError::Unavailable)?
        .to_owned();
    let binding = GatewayRouteBinding {
        route_id: row.route_id,
        gateway_revision_id: row.gateway_revision_id,
        path_prefix,
        methods,
        limits,
    };
    binding.validate()?;
    Ok(binding)
}

fn persisted_method(value: &str) -> Result<Method, GatewayEdgeError> {
    match value {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        "OPTIONS" => Ok(Method::OPTIONS),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}

fn canonical_request_path(path_and_query: &str) -> Result<&str, GatewayEdgeError> {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    if !path.starts_with('/') || path.contains(['#', '%']) || path.contains("//") {
        return Err(GatewayEdgeError::Contract("ambiguous request path"));
    }
    Ok(path)
}

fn route_matches(route: &GatewayRouteBinding, path: &str) -> bool {
    let public = route.public_path();
    path == public
        || path
            .strip_prefix(&public)
            .is_some_and(|suffix| suffix.starts_with('/'))
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
    /// Gateway whose active revision is being configured.
    pub gateway_id: Uuid,
    /// Active revision expected by the caller.
    pub expected_revision_id: Uuid,
    /// Typed values validated against the published agent schema.
    pub parameters: BTreeMap<ParameterName, ParameterValue>,
    /// Explicit inbound secret selections; no existing grants are copied.
    pub secret_selections: Vec<GatewaySecretSelection>,
}

/// Result of configuring one immutable gateway revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureGatewayResult {
    /// New active immutable revision, or the original result on replay.
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

impl PostgresGatewayManagement {
    /// Creates the management adapter over the control-plane connection pool.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

    /// Lists gateways visible in one project under forced RLS.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn list_project(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayManagementSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Project, project_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewaySummaryRow>(
            "SELECT id, project_id, repository_id, name, lifecycle, active_revision_id, updated_at
             FROM gateways
             WHERE project_id = $1 AND ($2::uuid IS NULL OR id > $2)
             ORDER BY id LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Returns gateway revisions and route intents only if the caller can read
    /// the exact durable gateway; no request bodies, parameters, or secrets are
    /// projected.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, absence, or persistence failure.
    pub async fn get(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
    ) -> Result<(GatewayManagementSummary, Vec<GatewayManagementRevision>), GatewayManagementError>
    {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let summary = sqlx::query_as::<_, GatewaySummaryRow>(
            "SELECT id, project_id, repository_id, name, lifecycle, active_revision_id, updated_at
             FROM gateways WHERE id = $1",
        )
        .bind(gateway_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        let revisions = sqlx::query_as::<_, GatewayRevisionRow>(
            "SELECT id, release_id, release_agent_id, handler_contract, exposure, secret_slots, mailbox_slots, created_at
             FROM gateway_revisions WHERE gateway_id = $1 ORDER BY created_at DESC, id DESC",
        )
        .bind(gateway_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut result = Vec::with_capacity(revisions.len());
        for revision in revisions {
            let routes = sqlx::query_as::<_, GatewayRouteRow>(
                "SELECT id, path, methods, enabled FROM gateway_routes
                 WHERE gateway_revision_id = $1 ORDER BY path, id",
            )
            .bind(revision.id)
            .fetch_all(&mut *tx)
            .await?;
            result.push(GatewayManagementRevision {
                id: revision.id,
                release_id: revision.release_id,
                release_agent_id: revision.release_agent_id,
                handler_contract: revision.handler_contract,
                exposure: revision.exposure,
                secret_slots: revision.secret_slots,
                mailbox_slots: revision.mailbox_slots,
                created_at: revision.created_at,
                routes: routes.into_iter().map(Into::into).collect(),
            });
        }
        tx.commit().await?;
        Ok((summary.into(), result))
    }

    /// Lists recent redacted ingress records after a gateway-level read check.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn ingress(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayIngressSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayIngressRow>(
            "SELECT id, gateway_revision_id, gateway_route_id, outcome, accepted_at, completed_at
             FROM gateway_invocations
             WHERE gateway_id = $1 AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
        )
        .bind(gateway_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Performs a capability-checked lifecycle compare-and-swap. A false
    /// result deliberately represents a stale or already-transitioned state.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn transition(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        expected: &str,
        next: &str,
    ) -> Result<bool, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let changed: bool =
            sqlx::query_scalar("SELECT gateway_transition_lifecycle($1, $2, $3, $4, $5)")
                .bind(gateway_id)
                .bind(expected)
                .bind(next)
                .bind(identity.user_id.as_uuid())
                .bind(identity.request_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Creates one immutable exact mailbox binding and its active publication
    /// grant after checking authority over both the gateway revision and the
    /// target agent instance.
    ///
    /// # Errors
    ///
    /// Returns an authorization, absence, bounded-input, or persistence error.
    // This transaction deliberately keeps authority, immutable binding, and
    // committed receipt/outbox work together so a grant cannot be published
    // without its corresponding product event.
    #[allow(clippy::too_many_lines)]
    pub async fn create_mailbox_binding(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_revision_id: Uuid,
        slot_key: &str,
        mailbox_id: Uuid,
        producer_id: &str,
    ) -> Result<GatewayMailboxBindingSummary, GatewayManagementError> {
        if !valid_gateway_slot(slot_key) || !valid_gateway_producer(producer_id) {
            return Err(GatewayManagementError::InvalidArgument);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::GatewayRevision, gateway_revision_id),
        )
        .await?;
        let instance_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT instance_id FROM mailboxes WHERE id = $1 AND state = 'active'",
        )
        .bind(mailbox_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::AgentInstance, instance_id),
        )
        .await?;
        let command_key = binding_command_key(identity, "create_gateway_mailbox_binding");
        let payload_hash = binding_payload_hash(
            "create_gateway_mailbox_binding",
            gateway_revision_id,
            slot_key,
            Some(mailbox_id),
            producer_id,
            None,
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_mailbox_binding_commands
                (command_key, operation, gateway_revision_id, slot_key, mailbox_id,
                 producer_id, payload_hash, actor_id, request_id)
             VALUES ($1, 'create_gateway_mailbox_binding', $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(gateway_revision_id)
        .bind(slot_key)
        .bind(mailbox_id)
        .bind(producer_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, GatewayMailboxBindingCommandRow>(
                "SELECT operation, gateway_revision_id, slot_key, mailbox_id,
                        producer_id, target_binding_id, payload_hash, actor_id,
                        result_binding_id
                 FROM gateway_mailbox_binding_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayManagementError::Unavailable)?;
            if prior.operation != "create_gateway_mailbox_binding"
                || prior.gateway_revision_id != gateway_revision_id
                || prior.slot_key.as_deref() != Some(slot_key)
                || prior.mailbox_id != Some(mailbox_id)
                || prior.producer_id.as_deref() != Some(producer_id)
                || prior.target_binding_id.is_some()
                || prior.payload_hash.as_slice() != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayManagementError::Conflict);
            }
            let binding_id = prior
                .result_binding_id
                .ok_or(GatewayManagementError::Unavailable)?;
            let row = load_mailbox_binding(&mut tx, binding_id)
                .await?
                .ok_or(GatewayManagementError::Unavailable)?;
            tx.commit().await?;
            return Ok(row.into());
        }
        // Binding lifecycle writes remain under the actor-scoped application
        // role. The runtime worker may settle publications, but cannot mint
        // or impersonate an operator's authority grant.
        let binding_id = Uuid::new_v4();
        let grant_id = Uuid::new_v4();
        let row = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "WITH revision AS (
                 SELECT id, gateway_id, project_id FROM gateway_revisions WHERE id = $1
             ), binding AS (
                 INSERT INTO gateway_mailbox_bindings
                    (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
                 SELECT $2, revision.id, revision.gateway_id, revision.project_id, $3, $4, $5, $6
                 FROM revision
                 RETURNING id, gateway_revision_id, mailbox_id, slot_key, producer_id, created_at
             ), new_grant AS (
                 INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
                 SELECT $7, binding.id, 'active', $6 FROM binding
                 RETURNING id, binding_id, status, granted_at, revoked_at
             )
             SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, new_grant.id AS grant_id, new_grant.status AS grant_status,
                    binding.created_at, new_grant.granted_at, new_grant.revoked_at
             FROM binding JOIN new_grant ON new_grant.binding_id = binding.id",
        )
        .bind(gateway_revision_id)
        .bind(binding_id)
        .bind(slot_key)
        .bind(mailbox_id)
        .bind(producer_id)
        .bind(identity.user_id.as_uuid())
        .bind(grant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        sqlx::query(
            "UPDATE gateway_mailbox_binding_commands
             SET result_binding_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(binding_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// Validates runtime values and explicit secret selections against the
    /// published release, then atomically creates and activates an immutable
    /// configured revision. Existing mailbox and secret grants are never
    /// copied; a new revision therefore remains fail-closed until callers
    /// explicitly create its bindings.
    ///
    /// # Errors
    ///
    /// Returns authorization, stale-target, bounded-input, idempotency, or
    /// persistence failures without exposing secret metadata.
    #[allow(clippy::too_many_lines)]
    pub async fn configure(
        &self,
        identity: &AuthenticatedIdentity,
        command: ConfigureGatewayRequest,
    ) -> Result<ConfigureGatewayResult, GatewayConfigureError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Gateway, command.gateway_id),
        )
        .await
        .map_err(|error| match error {
            GatewayManagementError::Denied => GatewayConfigureError::Denied,
            GatewayManagementError::Persistence(error) => GatewayConfigureError::Persistence(error),
            _ => GatewayConfigureError::Persistence(sqlx::Error::Protocol(
                "authorization lookup failed".into(),
            )),
        })?;
        let current = sqlx::query_as::<_, ConfigureRevisionRow>(
            "SELECT gateway.project_id, gateway.repository_id, gateway.active_revision_id,
                    gateway.lifecycle,
                    revision.release_id, revision.release_agent_id, revision.release_agent_key,
                    revision.handler_contract, revision.exposure, revision.secret_slots,
                    revision.mailbox_slots,
                    agent.parameter_schema, release.state AS release_state
             FROM gateways AS gateway
             JOIN gateway_revisions AS revision
               ON revision.gateway_id = gateway.id AND revision.id = $2
             LEFT JOIN release_agents AS agent ON agent.id = revision.release_agent_id
             LEFT JOIN releases AS release ON release.id = revision.release_id
             WHERE gateway.id = $1 AND revision.id = $2",
        )
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayConfigureError::NotFound)?;
        // The durable command ledger is consulted before the active-revision
        // CAS check. A successful retry must replay after another revision
        // becomes active without reactivating its original result.
        let release_id = current.release_id.ok_or(GatewayConfigureError::Stale)?;
        let release_agent_id = current
            .release_agent_id
            .ok_or(GatewayConfigureError::Stale)?;
        let declarations: Vec<ParameterDeclaration> = serde_json::from_value(
            current
                .parameter_schema
                .ok_or(GatewayConfigureError::InvalidArgument)?,
        )
        .map_err(|_| GatewayConfigureError::InvalidArgument)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(|_| GatewayConfigureError::InvalidArgument)?;
        let routes = sqlx::query_as::<_, ConfigureRouteRow>(
            "SELECT path, methods, enabled FROM gateway_routes
             WHERE gateway_revision_id = $1 ORDER BY path, id",
        )
        .bind(command.expected_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut slots = BTreeSet::new();
        for selection in &command.secret_selections {
            if !valid_gateway_slot(&selection.slot_key)
                || !slots.insert(selection.slot_key.clone())
                || selection.route_path.is_empty()
                || !valid_header_name(&selection.header_name)
            {
                return Err(GatewayConfigureError::InvalidArgument);
            }
            if !current
                .secret_slots
                .iter()
                .any(|slot| slot == &selection.slot_key)
            {
                return Err(GatewayConfigureError::InvalidArgument);
            }
            let route = routes
                .iter()
                .find(|route| route.path == selection.route_path);
            if route.is_none() {
                return Err(GatewayConfigureError::InvalidArgument);
            }
            self.require(
                &mut tx,
                identity,
                Permission::BindBrokered,
                ObjectRef::new(ObjectType::SecretImport, selection.import_id),
            )
            .await
            .map_err(|error| match error {
                GatewayManagementError::Persistence(error) => {
                    GatewayConfigureError::Persistence(error)
                }
                _ => GatewayConfigureError::Denied,
            })?;
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                   SELECT 1 FROM secret_imports imported
                   JOIN secret_grants granted ON granted.id = imported.grant_id
                   JOIN secrets owned ON owned.id = imported.secret_id
                   JOIN secret_versions version ON version.id = $2 AND version.secret_id = owned.id
                   WHERE imported.id = $1 AND imported.target_kind = 'project'
                     AND imported.target_id = $3 AND imported.status = 'active'
                     AND granted.status = 'active' AND owned.status = 'active'
                     AND version.status = 'active' AND version.revoked_at IS NULL
                     AND version.purged_at IS NULL
                     AND 'normal' = ANY(granted.phases)
                     AND 'brokered' = ANY(granted.delivery_modes)
                     AND 'brokered' = ANY(owned.allowed_delivery_modes)
                     AND cardinality(granted.destinations) = 0
                 )",
            )
            .bind(selection.import_id)
            .bind(selection.secret_version_id)
            .bind(current.project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !valid {
                return Err(GatewayConfigureError::NotFound);
            }
        }
        let payload_hash = configure_payload_hash(
            command.gateway_id,
            command.expected_revision_id,
            release_id,
            release_agent_id,
            parameters.hash().as_bytes(),
            &command.secret_selections,
        );
        let command_key = ReleaseCommandKey::derive(
            "configure_gateway",
            &[identity.idempotency_id.as_uuid().as_bytes()],
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_configure_commands
               (command_key, operation, gateway_id, expected_revision_id, payload_hash, actor_id, request_id)
             VALUES ($1, 'configure_gateway', $2, $3, $4, $5, $6)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, ConfigureCommandRow>(
                "SELECT gateway_id, expected_revision_id, payload_hash, actor_id, result_revision_id
                 FROM gateway_configure_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayConfigureError::Persistence(sqlx::Error::RowNotFound))?;
            if prior.gateway_id != command.gateway_id
                || prior.expected_revision_id != command.expected_revision_id
                || prior.payload_hash != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayConfigureError::Conflict);
            }
            let revision_id =
                prior
                    .result_revision_id
                    .ok_or(GatewayConfigureError::Persistence(sqlx::Error::Protocol(
                        "incomplete configuration command".into(),
                    )))?;
            tx.commit().await?;
            return Ok(ConfigureGatewayResult { revision_id });
        }
        if current.lifecycle != "enabled"
            || current.active_revision_id != Some(command.expected_revision_id)
            || current.release_state.as_deref() != Some("published")
        {
            return Err(GatewayConfigureError::Stale);
        }
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await?;
        // Serialize competing fresh keys on the aggregate before creating a
        // revision. The first winner changes the active revision; followers
        // then observe a clean stale result instead of a uniqueness error.
        let still_active: bool = sqlx::query_scalar(
            "SELECT lifecycle = 'enabled' AND active_revision_id = $2
             FROM gateways WHERE id = $1 FOR UPDATE",
        )
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !still_active {
            return Err(GatewayConfigureError::Stale);
        }
        // Reinstalling a declaration may select a previously configured
        // predecessor again. A fresh command must create a fresh authority
        // scope without borrowing the old revision's grants. Command replay
        // is resolved by the ledger above, using the stable payload hash.
        let mut revision_digest = Sha256::new();
        revision_digest.update(b"hephaestus.gateway.configured-revision.v1\0");
        revision_digest.update(payload_hash);
        revision_digest.update(command_key.as_bytes());
        let revision_hash: [u8; 32] = revision_digest.finalize().into();
        let revision_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_revisions
               (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
                release_agent_key, handler_contract, exposure, parameters, secret_slots,
                mailbox_slots, normalized_hash, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(revision_id)
        .bind(command.gateway_id)
        .bind(current.project_id)
        .bind(current.repository_id)
        .bind(release_id)
        .bind(release_agent_id)
        .bind(current.release_agent_key)
        .bind(current.handler_contract)
        .bind(current.exposure)
        .bind(
            serde_json::to_value(parameters.values())
                .map_err(|_| GatewayConfigureError::InvalidArgument)?,
        )
        .bind(&current.secret_slots)
        .bind(&current.mailbox_slots)
        .bind(revision_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        let mut cloned_routes = BTreeMap::new();
        for route in routes {
            let new_route = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO gateway_routes (id,gateway_revision_id,gateway_id,project_id,path,methods,enabled)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(new_route)
            .bind(revision_id)
            .bind(command.gateway_id)
            .bind(current.project_id)
            .bind(&route.path)
            .bind(&route.methods)
            .bind(route.enabled)
            .execute(&mut *tx)
            .await?;
            cloned_routes.insert(route.path, new_route);
        }
        for selection in &command.secret_selections {
            let binding_id = Uuid::new_v4();
            let selection_hash = secret_selection_hash(selection, revision_id);
            sqlx::query(
                "INSERT INTO gateway_secret_bindings
                   (id,gateway_id,gateway_revision_id,import_id,slot_key,secret_version_id,status,normalized_hash)
                 VALUES ($1,$2,$3,$4,$5,$6,'active',$7)",
            )
            .bind(binding_id)
            .bind(command.gateway_id)
            .bind(revision_id)
            .bind(selection.import_id)
            .bind(&selection.slot_key)
            .bind(selection.secret_version_id)
            .bind(selection_hash.as_slice())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO gateway_brokered_secret_rules
                   (id,binding_id,gateway_revision_id,gateway_route_id,header_name,normalized_hash)
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(Uuid::new_v4())
            .bind(binding_id)
            .bind(revision_id)
            .bind(cloned_routes[&selection.route_path])
            .bind(&selection.header_name)
            .bind(selection_hash.as_slice())
            .execute(&mut *tx)
            .await?;
        }
        let changed = sqlx::query(
            "UPDATE gateways SET active_revision_id = $2, updated_at = now()
             WHERE id = $1 AND active_revision_id = $3",
        )
        .bind(command.gateway_id)
        .bind(revision_id)
        .bind(command.expected_revision_id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(GatewayConfigureError::Stale);
        }
        sqlx::query(
            "UPDATE gateway_configure_commands SET result_revision_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ConfigureGatewayResult { revision_id })
    }

    /// Revokes the active publication grant while retaining immutable binding
    /// and publication provenance for inspection.
    ///
    /// # Errors
    ///
    /// Returns an authorization, absence, stale-state, or persistence error.
    // Revocation retains the immutable publication chain while atomically
    // recording the grant transition and its committed product event.
    #[allow(clippy::too_many_lines)]
    pub async fn revoke_mailbox_binding_grant(
        &self,
        identity: &AuthenticatedIdentity,
        binding_id: Uuid,
    ) -> Result<GatewayMailboxBindingSummary, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = sqlx::query_as::<_, GatewayMailboxBindingTargetRow>(
            "SELECT binding.gateway_revision_id, mailbox.instance_id
             FROM gateway_mailbox_bindings binding
             JOIN mailboxes mailbox ON mailbox.id = binding.mailbox_id
             WHERE binding.id = $1",
        )
        .bind(binding_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::GatewayRevision, target.gateway_revision_id),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::AgentInstance, target.instance_id),
        )
        .await?;
        let command_key = binding_command_key(identity, "revoke_gateway_mailbox_binding_grant");
        let payload_hash = binding_payload_hash(
            "revoke_gateway_mailbox_binding_grant",
            target.gateway_revision_id,
            "",
            None,
            "",
            Some(binding_id),
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_mailbox_binding_commands
                (command_key, operation, gateway_revision_id, target_binding_id,
                 payload_hash, actor_id, request_id)
             VALUES ($1, 'revoke_gateway_mailbox_binding_grant', $2, $3, $4, $5, $6)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(target.gateway_revision_id)
        .bind(binding_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, GatewayMailboxBindingCommandRow>(
                "SELECT operation, gateway_revision_id, slot_key, mailbox_id,
                        producer_id, target_binding_id, payload_hash, actor_id,
                        result_binding_id
                 FROM gateway_mailbox_binding_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayManagementError::Unavailable)?;
            if prior.operation != "revoke_gateway_mailbox_binding_grant"
                || prior.gateway_revision_id != target.gateway_revision_id
                || prior.target_binding_id != Some(binding_id)
                || prior.slot_key.is_some()
                || prior.mailbox_id.is_some()
                || prior.producer_id.is_some()
                || prior.payload_hash.as_slice() != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayManagementError::Conflict);
            }
            let result_binding_id = prior
                .result_binding_id
                .ok_or(GatewayManagementError::Unavailable)?;
            let row = load_mailbox_binding(&mut tx, result_binding_id)
                .await?
                .ok_or(GatewayManagementError::Unavailable)?;
            tx.commit().await?;
            return Ok(row.into());
        }
        let row = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "WITH updated AS (
                 UPDATE gateway_mailbox_binding_grants
                    SET status = 'revoked', revoked_at = now(), revoked_by = $2
                  WHERE binding_id = $1 AND status = 'active'
                 RETURNING id, binding_id, status, granted_at, revoked_at
             )
             SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, updated.id AS grant_id, updated.status AS grant_status,
                    binding.created_at, updated.granted_at, updated.revoked_at
             FROM gateway_mailbox_bindings binding JOIN updated ON updated.binding_id = binding.id",
        )
        .bind(binding_id)
        .bind(identity.user_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::Conflict)?;
        sqlx::query(
            "UPDATE gateway_mailbox_binding_commands
             SET result_binding_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(binding_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// Lists redacted exact bindings for one revision visible to the caller.
    ///
    /// # Errors
    ///
    /// Returns an authorization or persistence error.
    pub async fn mailbox_bindings(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_revision_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayMailboxBindingSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::GatewayRevision, gateway_revision_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, binding_grant.id AS grant_id, binding_grant.status AS grant_status,
                    binding.created_at, binding_grant.granted_at, binding_grant.revoked_at
             FROM gateway_mailbox_bindings binding
             JOIN gateway_mailbox_binding_grants binding_grant
               ON binding_grant.binding_id = binding.id
             WHERE binding.gateway_revision_id = $1 AND ($2::uuid IS NULL OR binding.id > $2)
             ORDER BY binding.id LIMIT $3",
        )
        .bind(gateway_revision_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Lists joined, redacted publication provenance for a gateway. It traces
    /// each publication through its exact authorization snapshot, delivery,
    /// and most-recent run while excluding payload/body/header/key/runtime-
    /// session and denial details.
    ///
    /// # Errors
    ///
    /// Returns an authorization or persistence error.
    pub async fn mailbox_publications(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayMailboxPublicationSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayMailboxPublicationManagementRow>(
            "SELECT publication.id, publication.invocation_id, publication.gateway_revision_id,
                    publication.binding_id, publication.grant_id, publication.mailbox_id,
                    publication.event_id, publication.slot_key, publication.outcome,
                    publication.accepted_at, publication.settled_at,
                    session.snapshot_id AS authorization_snapshot_id,
                    snapshot_binding.ordinal AS snapshot_binding_ordinal,
                    delivery.disposition AS delivery_disposition,
                    delivery.logical_attempt_count AS delivery_attempt_count,
                    delivery.terminal_at AS delivery_terminal_at,
                    attempt.id AS delivery_attempt_id, attempt.run_id,
                    run.state AS run_state, run.outcome AS run_outcome
             FROM gateway_mailbox_publications publication
             JOIN gateway_invocations invocation ON invocation.id = publication.invocation_id
             JOIN gateway_runtime_authority_sessions session
                ON session.id = publication.runtime_session_id
             LEFT JOIN gateway_authorization_snapshot_bindings snapshot_binding
                ON snapshot_binding.snapshot_id = session.snapshot_id
               AND snapshot_binding.binding_id = publication.binding_id
             LEFT JOIN mailbox_deliveries delivery ON delivery.event_id = publication.event_id
             LEFT JOIN LATERAL (
                SELECT id, run_id FROM mailbox_delivery_attempts
                 WHERE event_id = publication.event_id
                 ORDER BY attempt_number DESC, id DESC LIMIT 1
             ) attempt ON true
             LEFT JOIN runs run ON run.id = attempt.run_id
             WHERE invocation.gateway_id = $1 AND ($2::uuid IS NULL OR publication.id < $2)
             ORDER BY publication.id DESC LIMIT $3",
        )
        .bind(gateway_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
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
    lifecycle: String,
    release_id: Option<Uuid>,
    release_agent_id: Option<Uuid>,
    release_agent_key: Option<String>,
    handler_contract: String,
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
    exposure: String,
    secret_slots: Vec<String>,
    mailbox_slots: Vec<String>,
    created_at: OffsetDateTime,
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

#[allow(clippy::too_many_arguments)]
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
             mailbox_slots, normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
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
    sqlx::query("UPDATE gateways SET active_revision_id = $2, updated_at = now() WHERE id = $1")
        .bind(gateway_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(&mut **tx)
        .await?;
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
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("telegram"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let nested = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
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
        let row = ActiveRouteRow {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path: String::from("/telegram"),
            methods: vec![String::from("CONNECT")],
        };
        assert!(active_route(row, edge_limits()).is_err());
    }

    #[test]
    fn desired_configuration_revision_is_order_independent_and_tracks_cutover() {
        let first = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: String::from("first"),
            methods: BTreeSet::from([Method::POST]),
            limits: edge_limits(),
        };
        let second = GatewayRouteBinding {
            route_id: Uuid::new_v4(),
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
