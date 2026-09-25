//! Provider-neutral application contracts for the durable event boundary.
#![allow(missing_docs)] // Event DTO fields mirror generated transport contracts.

use async_trait::async_trait;
use identity_domain::{RequestId, UserId};
use std::error::Error;
use uuid::Uuid;

/// Durable event metadata returned after an idempotent mutation commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedMutation {
    /// Stable identifier of the committed application event.
    pub event_id: Uuid,
    /// Scope kind whose ordered cursor contains the event.
    pub scope_kind: String,
    /// Stable identifier of the event scope.
    pub scope_id: Uuid,
    /// Monotonic cursor within the event scope.
    pub cursor: i64,
    /// Monotonic version of the changed aggregate within the scope.
    pub aggregate_version: i64,
}

/// Provider-neutral failures while loading a committed mutation receipt.
#[derive(Debug, thiserror::Error)]
pub enum MutationReceiptError {
    /// No committed event matched the exact mutation identity and primary scope.
    #[error("committed mutation receipt is missing")]
    Missing,
    /// The configured receipt provider could not complete the lookup.
    #[error("mutation receipt provider failed")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl MutationReceiptError {
    /// Wraps a provider-specific failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

/// Reads the durable event committed for an idempotent mutation.
#[async_trait]
pub trait MutationReceiptReader: Send + Sync {
    /// Loads the latest event matching the exact occurrence, actor, aggregate,
    /// and primary scope kind.
    async fn load(
        &self,
        occurrence_id: RequestId,
        actor_id: UserId,
        aggregate_type: &str,
        primary_scope_kind: &str,
    ) -> Result<CommittedMutation, MutationReceiptError>;
}

/// Provider-neutral persisted product-event projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductEventRecord {
    /// Event identifier.
    pub id: Uuid,
    /// Scope kind and identifier.
    pub scope_kind: String,
    pub scope_id: Uuid,
    /// Ordered scope cursor.
    pub cursor: i64,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub aggregate_version: i64,
    pub event_type: String,
    pub schema_version: i32,
    pub change_kind: String,
    pub safe_state: Option<String>,
    pub related_id_one: Option<Uuid>,
    pub related_id_two: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub request_id: Option<Uuid>,
    pub occurred_at: time::OffsetDateTime,
}

/// Provider-neutral outbox port for product-event publication.
#[async_trait]
pub trait ProductEventOutbox: Send + Sync {
    /// Loads unpublished event projections up to `limit`.
    async fn pending(&self, limit: i64) -> Result<Vec<ProductEventRecord>, MutationReceiptError>;
    /// Marks an event published.
    async fn mark_published(&self, event_id: Uuid) -> Result<(), MutationReceiptError>;
    /// Records a retry failure.
    async fn mark_failed(&self, event_id: Uuid, message: &str) -> Result<(), MutationReceiptError>;
    /// Records an unrecoverable projection failure and removes it from retry.
    async fn dead_letter(&self, event_id: Uuid, reason: &str) -> Result<(), MutationReceiptError>;
}
