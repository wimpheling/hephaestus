use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, PlanningCatalog, PlanningCatalogSnapshot, PlanningRequest,
    PlatformPolicyObservation,
};
use recipe_domain::{ExternalVolume, ResourceDeclaration, VolumeSource};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{PostgresDeploymentRepository, catalog, planning_sources, repository_error};

/// Authenticated catalog reader with platform policy supplied by server composition.
///
/// No request can select policy or supply source metadata. A plan grants nothing;
/// install must repeat current checks and admission before any provider mutation.
#[derive(Clone)]
pub struct PostgresPlanningCatalog {
    repository: PostgresDeploymentRepository,
    platform: PlatformPolicyObservation,
}

impl PostgresPlanningCatalog {
    /// Creates a reader with the operator's checked configured platform policy.
    #[must_use]
    pub const fn new(pool: PgPool, platform: PlatformPolicyObservation) -> Self {
        Self {
            repository: PostgresDeploymentRepository::new(pool),
            platform,
        }
    }

    async fn authorize(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        request: &PlanningRequest,
    ) -> Result<(), DeploymentError> {
        self.repository
            .require(
                tx,
                identity,
                Permission::CanManage,
                ObjectRef::new(ObjectType::Project, request.project_id().as_uuid()),
            )
            .await?;
        for resource in &request.declaration().manifest().resources {
            if let ResourceDeclaration::Instance(instance) = resource {
                self.repository
                    .require(
                        tx,
                        identity,
                        Permission::CanUse,
                        ObjectRef::new(
                            ObjectType::ReleaseAgent,
                            instance.release.release_agent_id.as_uuid(),
                        ),
                    )
                    .await?;
            }
        }
        for (name, id) in request.external() {
            if !request
                .declaration()
                .manifest()
                .resources
                .iter()
                .any(|resource| {
                    matches!(resource, ResourceDeclaration::Volume(volume)
                    if &volume.name == name && matches!(volume.source, VolumeSource::External {}))
                })
            {
                return Err(DeploymentError::InvalidPlanningInput);
            }
            let object = ObjectRef::new(ObjectType::StateVolume, id.as_uuid());
            self.repository
                .require(tx, identity, Permission::CanRead, object)
                .await?;
            let bound = request
                .declaration()
                .manifest()
                .resources
                .iter()
                .any(|resource| {
                    matches!(resource, ResourceDeclaration::Instance(instance)
                    if instance.volume_bindings.iter().any(|binding| &binding.resource == name))
                });
            if bound {
                for permission in [Permission::CanAttach, Permission::CanGrantAgentCapability] {
                    self.repository
                        .require(tx, identity, permission, object)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl PlanningCatalog for PostgresPlanningCatalog {
    async fn load_for_plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PlanningRequest,
    ) -> Result<PlanningCatalogSnapshot, DeploymentError> {
        let mut tx = begin_actor_transaction(&self.repository.pool, identity)
            .await
            .map_err(repository_error)?;
        self.authorize(&mut tx, identity, request).await?;
        let mut releases = Vec::new();
        let mut sources = Vec::new();
        let mut seen = BTreeSet::new();
        for resource in &request.declaration().manifest().resources {
            let ResourceDeclaration::Instance(instance) = resource else {
                continue;
            };
            let pin = instance.release;
            if seen.insert((pin.release_id.as_uuid(), pin.release_agent_id.as_uuid())) {
                let (evidence, source) = catalog::load_release(&mut tx, pin).await?;
                sources.push(planning_sources::observe(
                    &evidence,
                    &source,
                    &self.platform,
                )?);
                releases.push(evidence.view());
            }
        }
        let mut external = BTreeMap::new();
        for (name, id) in request.external() {
            let (project, capacity, state, provisioning): (uuid::Uuid, i64, String, String) =
                sqlx::query_as(
                    "SELECT project_id, capacity_bytes, state, provisioning_state
                 FROM agent_instance_state_volumes WHERE id = $1",
                )
                .bind(id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(repository_error)?
                .ok_or(DeploymentError::Unavailable)?;
            if project != request.project_id().as_uuid()
                || !matches!(state.as_str(), "ready" | "attached")
                || provisioning != "ready"
            {
                return Err(DeploymentError::Unavailable);
            }
            external.insert(
                name.clone(),
                ExternalVolume {
                    id: *id,
                    capacity_bytes: u64::try_from(capacity)
                        .map_err(|_| DeploymentError::InvalidPlanningInput)?,
                },
            );
        }
        tx.commit().await.map_err(repository_error)?;
        Ok(PlanningCatalogSnapshot {
            releases,
            sources,
            external,
        })
    }
}
