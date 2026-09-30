//! Shared service recovery fixture types and authority construction.

use gateway_domain::GatewayLimits;
use gateway_postgres::PostgresGatewayEdgeAuthority;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub owner: Uuid,
    pub organization: Uuid,
    pub project: Uuid,
    pub gateway: Uuid,
    pub revision: Uuid,
    pub route: Uuid,
    pub service_instance: Option<Uuid>,
}

pub const fn authority(pool: sqlx::PgPool) -> PostgresGatewayEdgeAuthority {
    PostgresGatewayEdgeAuthority::new(
        pool,
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: Duration::from_secs(10),
        },
    )
}
