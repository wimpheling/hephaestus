//! Database pools and request construction for UI authority tests.

use gateway_domain::{
    GatewayLimits, GatewayScheme, TrustedRequestMetadata, UiGatewayAuthority, UiGatewayRequest,
    UiGatewayRequestKind,
};
use http::{HeaderMap, Method};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{net::IpAddr, str::FromStr, time::Duration};
use uuid::Uuid;

pub async fn observed_worker_pool(database_url: &str) -> PgPool {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse observed worker database URL")
        .application_name("ui-gateway-service-instance-blocker");
    PgPoolOptions::new()
        .max_connections(6)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("connect observed worker pool")
}

pub async fn observed_activity_pool(database_url: &str) -> PgPool {
    let options = PgConnectOptions::from_str(database_url)
        .expect("parse observer database URL")
        .application_name("ui-gateway-service-instance-observer");
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect privileged activity observer pool")
}

pub async fn wait_for_service_instance_wait(pool: &PgPool) {
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                   FROM pg_stat_activity
                  WHERE application_name = 'ui-gateway-service-instance-blocker'
                    AND wait_event_type = 'Lock'
             )",
        )
        .fetch_one(pool)
        .await
        .expect("observe service instance blocker wait");
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let activity: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT application_name, wait_event_type, query
           FROM pg_stat_activity
          WHERE application_name = 'ui-gateway-service-instance-blocker'",
    )
    .fetch_all(pool)
    .await
    .expect("inspect named service instance blocker activity");
    panic!("named gateway service instance blocker was not observed: {activity:?}");
}

pub const fn limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: 16 * 1024,
        max_response_body_bytes: 16 * 1024,
        max_request_headers: 64,
        max_response_headers: 64,
        max_path_and_query_bytes: 4096,
        execution_timeout: Duration::from_secs(5),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn authority(
    child_session_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    actor_id: Uuid,
    organization_id: Uuid,
    path: &str,
    request_kind: UiGatewayRequestKind,
    method: Method,
) -> UiGatewayAuthority {
    UiGatewayAuthority {
        child_session_id,
        actor_id,
        organization_id,
        installation_id,
        generation_id,
        canonical_request_path: path.to_owned(),
        request_kind,
        method,
    }
}

// The generated request body type is an indirect dependency; keep its
// default construction generic at this adapter boundary.
#[allow(clippy::default_trait_access)]
pub fn request(authority: UiGatewayAuthority) -> UiGatewayRequest {
    UiGatewayRequest {
        method: authority.method.clone(),
        request_path_and_query: authority.canonical_request_path.clone(),
        authority,
        headers: HeaderMap::new(),
        body: Default::default(),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Https,
            authority: "ui.heph.test".to_owned(),
            client_address: "127.0.0.1".parse::<IpAddr>().expect("loopback address"),
            request_id: Uuid::new_v4(),
        },
    }
}

pub async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

pub async fn app_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}
