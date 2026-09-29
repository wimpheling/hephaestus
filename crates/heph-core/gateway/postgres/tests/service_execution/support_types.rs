//! Shared execution-target fixture types and timing controls.

use gateway_domain::{
    GatewayExecutionTargetError, GatewayExecutionTargetResolver, GatewayServiceOwner,
};
use gateway_postgres::PostgresGatewayExecutionTargetResolver;
use uuid::Uuid;

#[derive(Clone)]
pub struct Fixture {
    pub project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub instance: Uuid,
    pub invocation: Uuid,
    pub session: Uuid,
    pub release: Uuid,
    pub owner: GatewayServiceOwner,
    pub secret_lease: Uuid,
}

#[derive(Clone, Copy)]
pub enum TimingWindow {
    Live,
    Near,
    Expired,
}

pub async fn assert_unavailable(
    resolver: &PostgresGatewayExecutionTargetResolver,
    fixture: &Fixture,
) {
    assert_eq!(
        resolver
            .resolve_execution_target(
                fixture.invocation,
                fixture.route,
                fixture.revision,
                &fixture.owner,
            )
            .await,
        Err(GatewayExecutionTargetError::Unavailable)
    );
}

#[derive(Clone, Copy)]
pub struct SecretContext {
    pub organization: Uuid,
    pub project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub owner_id: Uuid,
    pub secret: Uuid,
    pub version: Uuid,
    pub grant: Uuid,
    pub import: Uuid,
    pub binding: Uuid,
    pub rule: Uuid,
    pub secret_lease: Uuid,
    pub session: Uuid,
    pub invocation: Uuid,
}
