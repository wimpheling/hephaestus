//! Gateway mailbox publication authority and idempotent persistence.

use super::GatewayEdgeError;
use super::gateway_mailbox_payload::validate_gateway_mailbox_payload;
use mailbox_domain::{DeduplicationKey, MailboxEnvelope, MailboxEventId};
use sqlx::PgPool;
use uuid::Uuid;

/// Host-only request to publish one generic event through an exact gateway
/// mailbox slot. The caller never chooses the mailbox or producer identity.
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
