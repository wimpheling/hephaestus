//! Shared service acceptance types and issuer doubles.

use async_trait::async_trait;
use capability_domain::{
    AuthorityHash, RuntimeCredentialGeneration, RuntimeSessionId, RuntimeSessionStatus,
};
use gateway_domain::{GatewayLimits, GatewayRouteBinding};
use gateway_postgres::PostgresGatewayEdgeAuthority;
use http::Method;
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, RuntimeAuthorityError,
    StoredRuntimeSession,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use time::OffsetDateTime;
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Clone)]
pub struct RecordingIssuer {
    pub pool: sqlx::PgPool,
    pub calls: Arc<Mutex<Vec<&'static str>>>,
    pub entered: Option<Arc<Notify>>,
    pub release: Option<Arc<Notify>>,
}

pub struct FailingIssuer;

pub struct GhostIssuer;

pub struct TestPools {
    pub admin: sqlx::PgPool,
    pub worker: sqlx::PgPool,
}

pub type ActivityRow = (i32, String, String, String, Option<String>, Option<String>);

#[derive(Clone, Copy)]
pub struct Fixture {
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub service_instance: Option<Uuid>,
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for RecordingIssuer {
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        self.calls
            .lock()
            .expect("recording issuer mutex")
            .push("guest");
        let session = persist_recorded_session(&self.pool, request, false).await?;
        self.pause_if_requested().await;
        Ok(session)
    }

    async fn issue_gateway_service(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        self.calls
            .lock()
            .expect("recording issuer mutex")
            .push("service");
        let session = persist_recorded_session(&self.pool, request, true).await?;
        self.pause_if_requested().await;
        Ok(session)
    }
}

impl RecordingIssuer {
    async fn pause_if_requested(&self) {
        if let (Some(entered), Some(release)) = (&self.entered, &self.release) {
            entered.notify_one();
            release.notified().await;
        }
    }
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for FailingIssuer {
    async fn issue_gateway(
        &self,
        _request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Err(RuntimeAuthorityError::Persistence)
    }
}

#[async_trait]
impl GatewayRuntimeAuthorityIssuer for GhostIssuer {
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Ok(ghost_session(request))
    }

    async fn issue_gateway_service(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        Ok(ghost_session(request))
    }
}

pub fn build_authority(
    pool: sqlx::PgPool,
    issuer: Arc<dyn GatewayRuntimeAuthorityIssuer>,
) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(pool, limits())
        .with_runtime_authority(issuer, Duration::from_secs(300))
        .expect("valid session TTL")
}

pub const fn limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: 1024,
        max_response_body_bytes: 1024,
        max_request_headers: 16,
        max_response_headers: 16,
        max_path_and_query_bytes: 256,
        execution_timeout: Duration::from_secs(10),
    }
}

pub fn route(fixture: &Fixture, prefix: &str) -> GatewayRouteBinding {
    GatewayRouteBinding {
        route_id: fixture.route,
        exposure: gateway_domain::Exposure::Public,
        gateway_revision_id: fixture.revision,
        path_prefix: prefix.to_owned(),
        methods: BTreeSet::from([Method::GET]),
        limits: limits(),
    }
}

async fn persist_recorded_session(
    pool: &sqlx::PgPool,
    request: GatewayRuntimeSessionRequest,
    service: bool,
) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
    let hash = [7_u8; 32];
    let snapshot = request.invocation_id.as_uuid();
    let mode = if service {
        "host_mediated"
    } else {
        "guest_handoff"
    };
    let status = if service { "active" } else { "pending_handoff" };
    let credential_hash = if service { None } else { Some(hash.as_slice()) };
    sqlx::query(
        "INSERT INTO gateway_authorization_snapshots
            (id, invocation_id, gateway_id, gateway_revision_id,
             authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot)
    .bind(request.invocation_id.as_uuid())
    .bind(request.gateway_id)
    .bind(request.gateway_revision_id)
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .map_err(|_| RuntimeAuthorityError::Persistence)?;
    sqlx::query(
        "INSERT INTO gateway_runtime_authority_sessions
            (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
             identity_hash, snapshot_hash, issuance_generation, credential_hash,
             admission_mode, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8, $9, $10, $11, $12, $13)",
    )
    .bind(request.invocation_id.as_uuid())
    .bind(snapshot)
    .bind(request.invocation_id.as_uuid())
    .bind(request.gateway_id)
    .bind(request.gateway_revision_id)
    .bind(hash.as_slice())
    .bind(hash.as_slice())
    .bind(credential_hash)
    .bind(mode)
    .bind(status)
    .bind(request.issued_at)
    .bind(request.expires_at)
    .bind(None::<OffsetDateTime>)
    .execute(pool)
    .await
    .map_err(|_| RuntimeAuthorityError::Persistence)?;
    Ok(StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(request.invocation_id.as_uuid()),
        snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(snapshot),
        identity_hash: AuthorityHash::from_bytes(hash),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: if service {
            RuntimeSessionStatus::Active
        } else {
            RuntimeSessionStatus::PendingHandoff
        },
        issued_at: request.issued_at,
        expires_at: request.expires_at,
        acknowledged_at: None,
        revoked_at: None,
    })
}

fn ghost_session(request: GatewayRuntimeSessionRequest) -> StoredRuntimeSession {
    StoredRuntimeSession {
        id: RuntimeSessionId::from_uuid(Uuid::new_v4()),
        snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(Uuid::new_v4()),
        identity_hash: AuthorityHash::from_bytes([3_u8; 32]),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::Active,
        issued_at: request.issued_at,
        expires_at: request.expires_at,
        acknowledged_at: None,
        revoked_at: None,
    }
}
