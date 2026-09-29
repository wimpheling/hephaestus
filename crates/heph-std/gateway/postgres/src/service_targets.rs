//! `PostgreSQL` read adapter for persistent gateway service targets.

mod models;

use self::models::{OwnedServiceTargetRow, ServiceTargetRow};
use crate::service_ownership::ServiceInstanceRow;
use async_trait::async_trait;
use gateway_domain::{
    GatewayEdgeError, GatewayServiceIdentity, GatewayServiceInstanceKey,
    GatewayServiceInstanceLease, GatewayServiceInstancePage, GatewayServiceInstancePageResult,
    GatewayServiceOwnedTarget, GatewayServiceTargetPage, GatewayServiceTargetPageResult,
    GatewayServiceTargetStore,
};
use sqlx::PgPool;
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
                    active_revision.service_log_capture_mode AS active_service_log_capture_mode,
                    desired_revision.id AS desired_service_revision_row_id,
                    desired_revision.release_id AS desired_release_id,
                    desired_release.state AS desired_release_state,
                    desired_revision.service_loopback_port AS desired_service_loopback_port,
                    desired_revision.service_readiness_path AS desired_service_readiness_path,
                    desired_revision.service_health_path AS desired_service_health_path,
                    desired_revision.service_log_capture_mode AS desired_service_log_capture_mode
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
                    revision.service_health_path,
                    revision.service_log_capture_mode
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

    async fn list_service_instances(
        &self,
        page: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError> {
        page.validate()?;
        let rows = sqlx::query_as::<_, ServiceInstanceRow>(
            "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                    fencing_token, vm_id, state, lease_expires_at, heartbeat_at
               FROM gateway_service_instances
              WHERE owner_host_id = $1
                AND state <> 'cleaned'
                AND ($2::uuid IS NULL OR id > $2)
              ORDER BY id
              LIMIT $3",
        )
        .bind(&page.host_id)
        .bind(page.after)
        .bind(i64::from(page.limit) + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let mut instances = rows
            .into_iter()
            .map(|row| row.into_lease().map_err(|_| GatewayEdgeError::Unavailable))
            .collect::<Result<Vec<_>, _>>()?;
        let next_after = if instances.len() > usize::from(page.limit) {
            instances.pop();
            instances.last().map(|lease| lease.identity.instance_id)
        } else {
            None
        };
        Ok(GatewayServiceInstancePageResult {
            instances,
            next_after,
        })
    }
}
