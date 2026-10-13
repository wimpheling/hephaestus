use crate::{fixtures, seed, support};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentIntent, EffectClaim, ResourceAction,
    ResourceEffect,
};
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;

pub struct Ledger {
    pub seeded: support::Fixture,
    pub app: PgPool,
    pub workers: PgPool,
    pub recipe: Arc<PostgresDeploymentRepository>,
    pub identity: AuthenticatedIdentity,
    pub intent: DeploymentIntent,
    pub target: EffectClaim,
}
impl Ledger {
    pub async fn new(pool: &PgPool) -> Self {
        crate::private_migrations::checkpoint(112)
            .run(pool)
            .await
            .expect("112 before first provider IO");
        let seeded = seed::seed(pool).await;
        let app = fixtures::app_pool(pool).await;
        let workers = PgPoolOptions::new()
            .max_connections(6)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let recipe = Arc::new(PostgresDeploymentRepository::new(app.clone()));
        let identity = support::identity(seeded.maintainer);
        let intent = super::fixture::declaration(&seeded);
        let install = fixtures::install(&identity, intent.clone());
        let command = install.command;
        recipe
            .admit_install(&identity, install)
            .await
            .expect("actual original catalog admission");
        let target = recipe
            .claim_resource_effect(
                &identity,
                ResourceEffect {
                    command,
                    deployment_id: intent.id(),
                    resource: fixtures::key("data"),
                    expected_deployment_version: 0,
                    expected_resource_version: 0,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        &identity,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap();

        Self {
            seeded,
            app,
            workers,
            recipe,
            identity,
            intent,
            target,
        }
    }
}
