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
use release_domain::ReleaseId;
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeHandoffStore,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
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
               AND mailbox.state = 'active' AND instance.run_gate_open
               AND instance.state IN ('active', 'update_rejected')
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
               AND mailbox.state = 'active' AND instance.run_gate_open
               AND instance.state IN ('active', 'update_rejected')
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
    /// Builds a fresh read-only release mount for this invocation.
    ///
    /// # Errors
    ///
    /// Returns a safe edge error when the exact release tree cannot be built.
    fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[GatewayReleaseArtifact],
    ) -> Result<VmMount, GatewayEdgeError>;
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
            "SELECT agent.runtime_contract, session.issuance_generation, revision.release_id
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
        let release_mount = materializer.prepare(invocation_id, &artifacts)?;
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
            mounts: vec![release_mount],
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
    /// Supported handler contract.
    pub handler_contract: String,
    /// Declared exposure policy.
    pub exposure: String,
    /// Symbolic declared secret slot names only.
    pub secret_slots: Vec<String>,
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
            "SELECT id, release_id, handler_contract, exposure, secret_slots, created_at
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
                handler_contract: revision.handler_contract,
                exposure: revision.exposure,
                secret_slots: revision.secret_slots,
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
        tx.commit().await?;
        Ok(row.into())
    }

    /// Revokes the active publication grant while retaining immutable binding
    /// and publication provenance for inspection.
    ///
    /// # Errors
    ///
    /// Returns an authorization, absence, stale-state, or persistence error.
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
    handler_contract: String,
    exposure: String,
    secret_slots: Vec<String>,
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
        self.require_manage(&mut tx, identity, command.project_id)
            .await?;
        require_repository_boundary(&mut tx, &command).await?;

        let mut installed = Vec::with_capacity(config.gateways.len());
        for configured in config.gateways {
            let declaration = configured
                .to_declaration()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            let normalized_hash = declaration
                .validate()
                .map_err(|_| GatewayInstallError::Unavailable)?;
            // A gateway is always released code, never a floating repository
            // command.  Resolve the symbolic manifest key before writing the
            // immutable revision so later dispatch cannot silently select a
            // different agent in the same release.
            let release_agent = resolve_release_agent(&mut tx, &command, &declaration).await?;
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
struct ReleaseAgentBindingRow {
    id: Uuid,
    key: String,
}

fn parse_manifest(source: &[u8]) -> Result<RepositoryGatewaysConfig, GatewayInstallError> {
    let parsed = parse_repository_gateways(source);
    parsed.config.ok_or(GatewayInstallError::InvalidManifest {
        diagnostics: parsed.diagnostics,
    })
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
            "SELECT EXISTS (SELECT 1 FROM releases WHERE id = $1 AND repository_id = $2)",
        )
        .bind(release_id.as_uuid())
        .bind(command.repository_id.as_uuid())
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
}
