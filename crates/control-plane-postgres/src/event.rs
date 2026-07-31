//! Authorized durable application-event snapshots and resumable reads.

use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use futures_util::Stream;
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::{pin::Pin, sync::Arc};
use time::OffsetDateTime;
use uuid::Uuid;

const MAX_SNAPSHOT_AGGREGATES: i64 = 10_000;

pub type EventWakeupStream = Pin<Box<dyn Stream<Item = ()> + Send>>;

#[async_trait]
pub trait EventWakeupSource: Send + Sync {
    async fn subscribe(&self) -> Result<EventWakeupStream, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Identity,
    Organization,
    Project,
    Repository,
    Run,
    AgentInstance,
}

impl ScopeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Repository => "repository",
            Self::Run => "run",
            Self::AgentInstance => "agent_instance",
        }
    }

    const fn permission_object(self) -> Option<&'static str> {
        match self {
            Self::Identity => None,
            Self::Organization => Some("organization"),
            Self::Project => Some("project"),
            Self::Repository => Some("repository"),
            Self::Run => Some("run"),
            Self::AgentInstance => Some("agent_instance"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventScope {
    pub kind: ScopeKind,
    pub id: Uuid,
}

pub struct ScopeSnapshot {
    pub committed_cursor: i64,
    pub retained_from_cursor: i64,
    pub aggregate_versions: Vec<AggregateVersion>,
}

#[derive(FromRow)]
pub struct AggregateVersion {
    #[sqlx(rename = "aggregate_type")]
    pub kind: String,
    #[sqlx(rename = "aggregate_id")]
    pub id: Uuid,
    #[sqlx(rename = "aggregate_version")]
    pub version: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct ApplicationEvent {
    pub id: Uuid,
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
    pub occurred_at: OffsetDateTime,
}

pub enum ReadResult {
    Events {
        committed_cursor: i64,
        values: Vec<ApplicationEvent>,
    },
    RetentionGap {
        requested_cursor: i64,
        earliest_available_cursor: i64,
        latest_committed_cursor: i64,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("event scope access is denied")]
    PermissionDenied,
    #[error("event cursor is invalid")]
    InvalidCursor,
    #[error("event persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("event notification subscription failed: {0}")]
    Notification(String),
    #[error("event snapshot exceeds the aggregate reference limit")]
    ResourceExhausted,
}

#[derive(Clone)]
pub struct EventApplication {
    pool: PgPool,
    wakeups: Arc<dyn EventWakeupSource>,
}

impl EventApplication {
    pub fn new(pool: PgPool, wakeups: Arc<dyn EventWakeupSource>) -> Self {
        Self { pool, wakeups }
    }

    pub async fn subscribe(&self) -> Result<EventWakeupStream, EventError> {
        self.wakeups
            .subscribe()
            .await
            .map_err(EventError::Notification)
    }

    pub async fn snapshot(
        &self,
        identity: &AuthenticatedIdentity,
        scope: EventScope,
    ) -> Result<ScopeSnapshot, EventError> {
        let mut transaction = self.snapshot_transaction(identity).await?;
        authorize(&mut transaction, identity, scope).await?;
        let (committed_cursor, retained_from_cursor) = bounds(&mut transaction, scope).await?;
        let aggregate_versions = sqlx::query_as::<_, AggregateVersion>(
            "SELECT aggregate_type, aggregate_id, aggregate_version
             FROM application_aggregate_versions
             WHERE scope_kind = $1 AND scope_id = $2
             ORDER BY aggregate_type, aggregate_id
             LIMIT $3",
        )
        .bind(scope.kind.as_str())
        .bind(scope.id)
        .bind(MAX_SNAPSHOT_AGGREGATES + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(EventError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(EventError::Persistence)?;
        if aggregate_versions.len()
            > usize::try_from(MAX_SNAPSHOT_AGGREGATES).expect("positive constant fits usize")
        {
            return Err(EventError::ResourceExhausted);
        }
        Ok(ScopeSnapshot {
            committed_cursor,
            retained_from_cursor,
            aggregate_versions,
        })
    }

    pub async fn read_after(
        &self,
        identity: &AuthenticatedIdentity,
        scope: EventScope,
        cursor: i64,
        limit: i64,
    ) -> Result<ReadResult, EventError> {
        if cursor < 0 || !(1..=100).contains(&limit) {
            return Err(EventError::InvalidCursor);
        }
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(EventError::Persistence)?;
        authorize(&mut transaction, identity, scope).await?;
        let (committed_cursor, retained_from_cursor) = bounds(&mut transaction, scope).await?;
        if cursor.saturating_add(1) < retained_from_cursor {
            transaction
                .commit()
                .await
                .map_err(EventError::Persistence)?;
            return Ok(ReadResult::RetentionGap {
                requested_cursor: cursor,
                earliest_available_cursor: retained_from_cursor,
                latest_committed_cursor: committed_cursor,
            });
        }
        let values = sqlx::query_as::<_, ApplicationEvent>(
            "SELECT id, cursor, aggregate_type, aggregate_id, aggregate_version,
                    event_type, schema_version, change_kind, safe_state,
                    related_id_one, related_id_two, actor_id, request_id, occurred_at
             FROM application_events
             WHERE scope_kind = $1 AND scope_id = $2 AND cursor > $3
             ORDER BY cursor LIMIT $4",
        )
        .bind(scope.kind.as_str())
        .bind(scope.id)
        .bind(cursor)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(EventError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(EventError::Persistence)?;
        Ok(ReadResult::Events {
            committed_cursor,
            values,
        })
    }

    async fn snapshot_transaction<'a>(
        &'a self,
        identity: &AuthenticatedIdentity,
    ) -> Result<Transaction<'a, Postgres>, EventError> {
        let mut transaction = self.pool.begin().await.map_err(EventError::Persistence)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(EventError::Persistence)?;
        sqlx::query(
            "SELECT set_config('hephaestus.actor_id', $1, true),
                    set_config('hephaestus.subject_type', 'user', true),
                    set_config('hephaestus.request_id', $2, true)",
        )
        .bind(identity.user_id.to_string())
        .bind(identity.request_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(EventError::Persistence)?;
        Ok(transaction)
    }
}

async fn authorize(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    scope: EventScope,
) -> Result<(), EventError> {
    let allowed = if scope.kind == ScopeKind::Identity {
        scope.id == identity.user_id.as_uuid()
    } else {
        sqlx::query_scalar::<_, bool>(
            "SELECT check_permission(
                'user', hephaestus_actor_id(), 'can_read', $1, $2::text
             ) = 1",
        )
        .bind(
            scope
                .kind
                .permission_object()
                .ok_or(EventError::PermissionDenied)?,
        )
        .bind(scope.id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(EventError::Persistence)?
    };
    if allowed {
        Ok(())
    } else {
        Err(EventError::PermissionDenied)
    }
}

async fn bounds(
    transaction: &mut Transaction<'_, Postgres>,
    scope: EventScope,
) -> Result<(i64, i64), EventError> {
    Ok(sqlx::query_as::<_, (i64, i64)>(
        "SELECT committed_cursor, retained_from_cursor
         FROM application_event_scopes
         WHERE scope_kind = $1 AND scope_id = $2",
    )
    .bind(scope.kind.as_str())
    .bind(scope.id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(EventError::Persistence)?
    .unwrap_or((0, 1)))
}
