//! Project enumeration for service-log maintenance.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceLogMaintenanceError, GatewayServiceLogMaintenanceProjectPage,
    GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceProjects,
};
use uuid::Uuid;

use super::{PostgresGatewayServiceLogStore, maintenance_storage};

#[async_trait]
impl GatewayServiceLogMaintenanceProjects for PostgresGatewayServiceLogStore {
    async fn list_projects(
        &self,
        page: GatewayServiceLogMaintenanceProjectPage,
    ) -> Result<GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceError>
    {
        page.validate()?;
        let mut projects = sqlx::query_scalar::<_, Uuid>(
            "SELECT project_id
               FROM gateway_service_log_project_usage
              WHERE ($1::uuid IS NULL OR project_id > $1)
              ORDER BY project_id
              LIMIT $2",
        )
        .bind(page.after)
        .bind(i64::from(page.limit) + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(maintenance_storage)?;
        let next_after = if projects.len() > usize::from(page.limit) {
            projects.pop();
            projects.last().copied()
        } else {
            None
        };
        Ok(GatewayServiceLogMaintenanceProjectPageResult {
            projects,
            next_after,
        })
    }
}
