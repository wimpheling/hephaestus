//! `PostgreSQL` read adapter for persistent gateway service targets.

use crate::service_ownership::ServiceInstanceRow;
use async_trait::async_trait;
use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayEdgeError, GatewayServiceIdentity, GatewayServiceInstanceKey,
    GatewayServiceInstanceLease, GatewayServiceOwnedTarget, GatewayServiceRevisionTarget,
    GatewayServiceTarget, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    GatewayServiceTargetStore,
};
use sqlx::{FromRow, PgPool};
use std::convert::TryFrom;
use uuid::Uuid;

/// Worker-role `PostgreSQL` adapter for lifecycle-owned service target reads.
#[derive(Clone)]
pub struct PostgresGatewayServiceTargets {
    pool: PgPool,
}

impl PostgresGatewayServiceTargets {
    /// Creates a read adapter over the application worker pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayServiceTargetStore for PostgresGatewayServiceTargets {
    async fn list_service_targets(
        &self,
        page: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError> {
        page.validate()?;
        let rows = sqlx::query_as::<_, ServiceTargetRow>(
            "SELECT gateway.id AS gateway_id,
                    gateway.lifecycle,
                    gateway.active_revision_id,
                    gateway.desired_service_revision_id,
                    active_revision.id AS active_service_revision_id,
                    active_revision.release_id AS active_release_id,
                    active_release.state AS active_release_state,
                    active_revision.service_loopback_port AS active_service_loopback_port,
                    active_revision.service_readiness_path AS active_service_readiness_path,
                    active_revision.service_health_path AS active_service_health_path,
                    desired_revision.id AS desired_service_revision_row_id,
                    desired_revision.release_id AS desired_release_id,
                    desired_release.state AS desired_release_state,
                    desired_revision.service_loopback_port AS desired_service_loopback_port,
                    desired_revision.service_readiness_path AS desired_service_readiness_path,
                    desired_revision.service_health_path AS desired_service_health_path
               FROM gateways AS gateway
               LEFT JOIN gateway_revisions AS active_revision
                 ON active_revision.id = gateway.active_revision_id
                AND active_revision.gateway_id = gateway.id
                AND active_revision.handler_contract = 'http.service.v1'
               LEFT JOIN releases AS active_release
                 ON active_release.id = active_revision.release_id
               LEFT JOIN gateway_revisions AS desired_revision
                 ON desired_revision.id = gateway.desired_service_revision_id
                AND desired_revision.gateway_id = gateway.id
                AND desired_revision.handler_contract = 'http.service.v1'
               LEFT JOIN releases AS desired_release
                 ON desired_release.id = desired_revision.release_id
              WHERE gateway.lifecycle = 'enabled'
                AND (active_revision.id IS NOT NULL OR desired_revision.id IS NOT NULL)
                AND ($1::uuid IS NULL OR gateway.id > $1)
              ORDER BY gateway.id
              LIMIT $2",
        )
        .bind(page.after)
        .bind(i64::from(page.limit) + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;

        let mut rows = rows;
        let next_after = if rows.len() > usize::from(page.limit) {
            rows.pop();
            rows.last().map(|row| row.gateway_id)
        } else {
            None
        };
        let targets = rows
            .into_iter()
            .map(ServiceTargetRow::try_into_target)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GatewayServiceTargetPageResult {
            targets,
            next_after,
        })
    }

    async fn get_service_target(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError> {
        if gateway_id.is_nil() || revision_id.is_nil() {
            return Err(GatewayEdgeError::Contract(
                "gateway service target identity must not be nil",
            ));
        }
        let row = sqlx::query_as::<_, OwnedServiceTargetRow>(
            "SELECT gateway.id AS gateway_id,
                    gateway.lifecycle,
                    gateway.active_revision_id,
                    gateway.desired_service_revision_id,
                    revision.id AS revision_id,
                    revision.release_id,
                    release.state AS release_state,
                    revision.service_loopback_port,
                    revision.service_readiness_path,
                    revision.service_health_path
               FROM gateways AS gateway
               JOIN gateway_revisions AS revision
                 ON revision.gateway_id = gateway.id
                AND revision.id = $2
                AND revision.handler_contract = 'http.service.v1'
               LEFT JOIN releases AS release ON release.id = revision.release_id
              WHERE gateway.id = $1",
        )
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        row.map(OwnedServiceTargetRow::try_into_target).transpose()
    }

    async fn count_accepted_service_invocations(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<u64, GatewayEdgeError> {
        if gateway_id.is_nil() || revision_id.is_nil() {
            return Err(GatewayEdgeError::Contract(
                "gateway service target identity must not be nil",
            ));
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM gateway_invocations AS invocation
               JOIN gateway_revisions AS revision
                 ON revision.id = invocation.gateway_revision_id
                AND revision.gateway_id = invocation.gateway_id
                AND revision.handler_contract = 'http.service.v1'
              WHERE invocation.gateway_id = $1
                AND invocation.gateway_revision_id = $2
                AND invocation.outcome = 'accepted'",
        )
        .bind(gateway_id)
        .bind(revision_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        u64::try_from(count).map_err(|_| GatewayEdgeError::Unavailable)
    }

    async fn count_accepted_service_invocations_for_instance(
        &self,
        key: GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError> {
        if key.identity.instance_id.is_nil()
            || key.identity.gateway_id.is_nil()
            || key.identity.revision_id.is_nil()
            || key.fencing_token <= 0
        {
            return Err(GatewayEdgeError::Contract(
                "gateway service instance key is invalid",
            ));
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)::bigint
               FROM gateway_invocations AS invocation
               JOIN gateway_revisions AS revision
                 ON revision.id = invocation.gateway_revision_id
                AND revision.gateway_id = invocation.gateway_id
                AND revision.handler_contract = 'http.service.v1'
              WHERE invocation.gateway_id = $1
                AND invocation.gateway_revision_id = $2
                AND invocation.outcome = 'accepted'
                AND invocation.service_instance_id = $3
                AND invocation.service_instance_fencing_token = $4",
        )
        .bind(key.identity.gateway_id)
        .bind(key.identity.revision_id)
        .bind(key.identity.instance_id)
        .bind(key.fencing_token)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        u64::try_from(count).map_err(|_| GatewayEdgeError::Unavailable)
    }

    async fn get_service_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError> {
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
        {
            return Err(GatewayEdgeError::Contract(
                "gateway service instance identity must not be nil",
            ));
        }
        let row = sqlx::query_as::<_, ServiceInstanceRow>(
            "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                    fencing_token, vm_id, state, lease_expires_at, heartbeat_at
               FROM gateway_service_instances
              WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
        )
        .bind(identity.instance_id)
        .bind(identity.gateway_id)
        .bind(identity.revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        row.map(|value| {
            value
                .into_lease()
                .map_err(|_| GatewayEdgeError::Unavailable)
        })
        .transpose()
    }
}

#[derive(Debug, FromRow)]
struct ServiceTargetRow {
    gateway_id: Uuid,
    lifecycle: String,
    active_revision_id: Option<Uuid>,
    desired_service_revision_id: Option<Uuid>,
    active_service_revision_id: Option<Uuid>,
    active_release_id: Option<Uuid>,
    active_release_state: Option<String>,
    active_service_loopback_port: Option<i32>,
    active_service_readiness_path: Option<String>,
    active_service_health_path: Option<String>,
    desired_service_revision_row_id: Option<Uuid>,
    desired_release_id: Option<Uuid>,
    desired_release_state: Option<String>,
    desired_service_loopback_port: Option<i32>,
    desired_service_readiness_path: Option<String>,
    desired_service_health_path: Option<String>,
}

impl ServiceTargetRow {
    fn try_into_target(self) -> Result<GatewayServiceTarget, GatewayEdgeError> {
        Ok(GatewayServiceTarget {
            gateway_id: self.gateway_id,
            lifecycle: self.lifecycle,
            active_revision_id: self.active_revision_id,
            desired_service_revision_id: self.desired_service_revision_id,
            active_service_revision: service_revision(
                self.active_service_revision_id,
                self.active_release_id,
                self.active_release_state,
                self.active_service_loopback_port,
                self.active_service_readiness_path,
                self.active_service_health_path,
            )?,
            desired_service_revision: service_revision(
                self.desired_service_revision_row_id,
                self.desired_release_id,
                self.desired_release_state,
                self.desired_service_loopback_port,
                self.desired_service_readiness_path,
                self.desired_service_health_path,
            )?,
        })
    }
}

#[derive(Debug, FromRow)]
struct OwnedServiceTargetRow {
    gateway_id: Uuid,
    lifecycle: String,
    active_revision_id: Option<Uuid>,
    desired_service_revision_id: Option<Uuid>,
    revision_id: Uuid,
    release_id: Option<Uuid>,
    release_state: Option<String>,
    service_loopback_port: Option<i32>,
    service_readiness_path: Option<String>,
    service_health_path: Option<String>,
}

impl OwnedServiceTargetRow {
    fn try_into_target(self) -> Result<GatewayServiceOwnedTarget, GatewayEdgeError> {
        let revision = service_revision(
            Some(self.revision_id),
            self.release_id,
            self.release_state,
            self.service_loopback_port,
            self.service_readiness_path,
            self.service_health_path,
        )?
        .ok_or(GatewayEdgeError::Unavailable)?;
        Ok(GatewayServiceOwnedTarget {
            gateway_id: self.gateway_id,
            lifecycle: self.lifecycle,
            active_revision_id: self.active_revision_id,
            desired_service_revision_id: self.desired_service_revision_id,
            revision,
        })
    }
}

fn service_revision(
    revision_id: Option<Uuid>,
    release_id: Option<Uuid>,
    release_state: Option<String>,
    loopback_port: Option<i32>,
    readiness_path: Option<String>,
    health_path: Option<String>,
) -> Result<Option<GatewayServiceRevisionTarget>, GatewayEdgeError> {
    let Some(revision_id) = revision_id else {
        return Ok(None);
    };
    let loopback_port = loopback_port
        .and_then(|port| u16::try_from(port).ok())
        .ok_or(GatewayEdgeError::Unavailable)?;
    let readiness_path =
        ServiceProbePath::parse(readiness_path.ok_or(GatewayEdgeError::Unavailable)?)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
    let health_path = ServiceProbePath::parse(health_path.ok_or(GatewayEdgeError::Unavailable)?)
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let service = GatewayServiceConfig::new(loopback_port, readiness_path, health_path)
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let publication_eligible = release_state.as_deref() == Some("published");
    Ok(Some(GatewayServiceRevisionTarget {
        revision_id,
        release_id,
        release_state,
        publication_eligible,
        service,
    }))
}
