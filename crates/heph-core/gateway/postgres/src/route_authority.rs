//! Gateway route selection, active service admission, and connection ownership.

use super::{GatewayEdgeError, GatewayLimits, GatewayRouteBinding, PostgresGatewayEdgeAuthority};
use gateway_domain::Exposure;
use http::Method;
use sqlx::{Postgres, pool::PoolConnection};
use time::OffsetDateTime;
use uuid::Uuid;

/// Owns an adapter read connection until a successful query has completed.
///
/// `SQLx` 0.8.6 returns a checked-out pool connection from `Drop` through a
/// spawned task. If the query future is cancelled while `PostgreSQL` is still
/// executing it, that return task can leave the backend alive during pool
/// shutdown. Marking only the unsuccessful path for close-on-drop makes the
/// cancellation/error path deterministic while successful reads still return
/// their connection to the pool.
struct ActiveRoutesConnection {
    connection: Option<PoolConnection<Postgres>>,
}

impl ActiveRoutesConnection {
    const fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection: Some(connection),
        }
    }

    const fn connection_mut(&mut self) -> &mut PoolConnection<Postgres> {
        self.connection
            .as_mut()
            .expect("active route connection remains owned")
    }

    fn take(mut self) -> PoolConnection<Postgres> {
        self.connection
            .take()
            .expect("successful active route query takes its connection")
    }
}

impl Drop for ActiveRoutesConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.as_mut() {
            connection.close_on_drop();
        }
    }
}

impl PostgresGatewayEdgeAuthority {
    pub(super) async fn service_admission_binding(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        accepted: &AcceptedInvocationRow,
    ) -> Result<(Uuid, i64), GatewayEdgeError> {
        let instance = sqlx::query_as::<_, ServiceAdmissionRow>(
            "SELECT instance.id, instance.fencing_token,
                    instance.lease_expires_at
               FROM gateway_service_instances AS instance
              WHERE instance.gateway_id = $1
                AND instance.revision_id = $2
                AND instance.state = 'ready'
              FOR UPDATE",
        )
        .bind(accepted.gateway_id)
        .bind(accepted.gateway_revision_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .ok_or(GatewayEdgeError::Unavailable)?;

        let publication_eligible: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1
                   FROM gateway_revisions AS revision
                   JOIN releases AS release
                     ON release.id = revision.release_id
                    AND release.repository_id = revision.repository_id
                  WHERE revision.id = $1
                    AND revision.gateway_id = $2
                    AND revision.handler_contract = 'http.service.v1'
                    AND release.state = 'published'
                  FOR UPDATE OF release
             )",
        )
        .bind(accepted.gateway_revision_id)
        .bind(accepted.gateway_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        if !publication_eligible {
            return Err(GatewayEdgeError::Unavailable);
        }

        // The instance lock may have waited behind a lifecycle update and the
        // publication lock may have waited behind revocation. Check the lease
        // against a fresh database clock immediately before insertion.
        let database_now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if instance.lease_expires_at <= database_now {
            return Err(GatewayEdgeError::Unavailable);
        }
        Ok((instance.id, instance.fencing_token))
    }

    pub(super) async fn lock_authoritative_route(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        route: &GatewayRouteBinding,
    ) -> Result<AcceptedInvocationRow, GatewayEdgeError> {
        // Resolve the aggregate owner before taking its lock. All promotion,
        // lease, and acceptance paths lock gateway before its service
        // instance, so a promotion cannot commit between this lock and the
        // authoritative route check below.
        let gateway_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT gateway_id
               FROM gateway_routes
              WHERE id = $1 AND gateway_revision_id = $2",
        )
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        let Some(gateway_id) = gateway_id else {
            return Err(GatewayEdgeError::Unavailable);
        };
        let gateway_exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
                .bind(gateway_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| GatewayEdgeError::Unavailable)?;
        if gateway_exists.is_none() {
            return Err(GatewayEdgeError::Unavailable);
        }

        sqlx::query_as::<_, AcceptedInvocationRow>(
            "SELECT route.gateway_id, route.gateway_revision_id,
                    revision.handler_contract
               FROM gateway_routes AS route
               JOIN gateways AS gateway
                 ON gateway.id = route.gateway_id
               JOIN gateway_revisions AS revision
                 ON revision.id = route.gateway_revision_id
                AND revision.gateway_id = route.gateway_id
              WHERE route.id = $1
                AND route.gateway_revision_id = $2
                AND route.enabled
                AND gateway.lifecycle = 'enabled'
                AND gateway.active_revision_id = route.gateway_revision_id
                AND revision.exposure = 'public'",
        )
        .bind(route.route_id)
        .bind(route.gateway_revision_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .ok_or(GatewayEdgeError::Unavailable)
    }

    pub(super) async fn active_routes(&self) -> Result<Vec<GatewayRouteBinding>, GatewayEdgeError> {
        let mut connection = ActiveRoutesConnection::new(
            self.pool
                .acquire()
                .await
                .map_err(|_| GatewayEdgeError::Unavailable)?,
        );
        let rows = sqlx::query_as::<_, ActiveRouteRow>(
            "SELECT route.id AS route_id, route.gateway_revision_id, route.path, route.methods,
                    revision.exposure
             FROM gateway_routes AS route
             JOIN gateways AS gateway ON gateway.id = route.gateway_id
             JOIN gateway_revisions AS revision
               ON revision.id = route.gateway_revision_id
              AND revision.gateway_id = route.gateway_id
             WHERE gateway.lifecycle = 'enabled'
               AND gateway.active_revision_id = route.gateway_revision_id
               AND route.enabled
               AND revision.exposure = 'public'
             ORDER BY route.path, route.id",
        )
        .fetch_all(&mut **connection.connection_mut())
        .await
        .map_err(|_| GatewayEdgeError::Unavailable)?;
        // SQLx 0.8.6 normally returns `PoolConnection` from `Drop` through a
        // detached task. Await the successful path so a later pool close
        // cannot race this adapter's connection return; the cancellation and
        // error path still uses `close_on_drop` from `ActiveRoutesConnection`.
        let mut connection = connection.take();
        connection.return_to_pool().await;
        rows.into_iter()
            .map(|row| active_route(row, self.limits))
            .collect()
    }
}

#[derive(sqlx::FromRow)]
pub struct ActiveRouteRow {
    pub(crate) route_id: Uuid,
    pub(crate) gateway_revision_id: Uuid,
    pub(crate) path: String,
    pub(crate) methods: Vec<String>,
    pub(crate) exposure: String,
}

#[derive(sqlx::FromRow)]
pub struct AcceptedInvocationRow {
    pub(crate) gateway_id: Uuid,
    pub(crate) gateway_revision_id: Uuid,
    pub(crate) handler_contract: String,
}

#[derive(sqlx::FromRow)]
struct ServiceAdmissionRow {
    id: Uuid,
    fencing_token: i64,
    lease_expires_at: OffsetDateTime,
}

pub fn active_route(
    row: ActiveRouteRow,
    limits: GatewayLimits,
) -> Result<GatewayRouteBinding, GatewayEdgeError> {
    let methods = row
        .methods
        .into_iter()
        .map(|method| persisted_method(&method))
        .collect::<Result<_, _>>()?;
    let path_prefix = row
        .path
        .strip_prefix('/')
        .ok_or(GatewayEdgeError::Unavailable)?
        .to_owned();
    let binding = GatewayRouteBinding {
        route_id: row.route_id,
        gateway_revision_id: row.gateway_revision_id,
        exposure: match row.exposure.as_str() {
            "public" => Exposure::Public,
            "heph_authenticated" => Exposure::HephAuthenticated,
            _ => return Err(GatewayEdgeError::Unavailable),
        },
        path_prefix,
        methods,
        limits,
    };
    binding.validate()?;
    Ok(binding)
}

fn persisted_method(value: &str) -> Result<Method, GatewayEdgeError> {
    match value {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        "HEAD" => Ok(Method::HEAD),
        "OPTIONS" => Ok(Method::OPTIONS),
        _ => Err(GatewayEdgeError::Unavailable),
    }
}

pub fn canonical_request_path(path_and_query: &str) -> Result<&str, GatewayEdgeError> {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    if !path.starts_with('/') || path.contains(['#', '%']) || path.contains("//") {
        return Err(GatewayEdgeError::Contract("ambiguous request path"));
    }
    Ok(path)
}

pub fn route_matches(route: &GatewayRouteBinding, path: &str) -> bool {
    let public = route.public_path();
    path == public
        || path
            .strip_prefix(&public)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
