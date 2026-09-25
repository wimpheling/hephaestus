//! `PostgreSQL` durable ownership for long-lived gateway service instances.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError,
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

mod claim;
mod helpers;
mod recovery;
mod resolution;
mod transitions;

pub use helpers::ServiceInstanceRow;

/// `PostgreSQL` adapter for gateway service ownership and fencing.
#[derive(Clone)]
pub struct PostgresGatewayServiceOwnership {
    pool: PgPool,
}

impl PostgresGatewayServiceOwnership {
    /// Creates an ownership adapter over the worker-authorized pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayServiceOwnership for PostgresGatewayServiceOwnership {
    async fn claim_new(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        claim::claim_new(&self.pool, gateway_id, revision_id, owner, lease_duration).await
    }

    async fn renew(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        claim::renew(&self.pool, lease, owner, lease_duration).await
    }

    async fn claim_expired(
        &self,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
        limit: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        recovery::claim_expired(&self.pool, owner, lease_duration, limit).await
    }

    async fn mark_stopping(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transitions::mark_stopping(&self.pool, lease, owner).await
    }

    async fn mark_starting(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transitions::mark_starting(&self.pool, lease, owner).await
    }

    async fn mark_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transitions::mark_ready(&self.pool, lease, owner).await
    }

    async fn mark_draining(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transitions::mark_draining(&self.pool, lease, owner).await
    }

    async fn promote_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        transitions::promote_ready(&self.pool, lease, owner).await
    }

    async fn mark_cleaned(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        transitions::mark_cleaned(&self.pool, lease, owner).await
    }
}
