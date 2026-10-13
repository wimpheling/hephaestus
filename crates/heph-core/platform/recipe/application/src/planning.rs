use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_domain::{ExternalVolume, ReleaseCatalogEntry, ResourceDeclaration};
use serde::Serialize;

use crate::{DeploymentError, DeploymentIntent, PlanningRequest, SourceObservation};

/// Trusted catalog result. This type deliberately has no transport deserializer.
///
/// Public fields permit adapter construction; they are not proof of authorization.
pub struct PlanningCatalogSnapshot {
    /// Exact published schemas and released binding ceilings.
    pub releases: Vec<ReleaseCatalogEntry>,
    /// Authorized same-project external resource facts.
    pub external: BTreeMap<CapabilitySlotKey, ExternalVolume>,
    /// Checked source execution observations, outside frozen recipe hashes.
    pub sources: Vec<SourceObservation>,
}

/// Authenticated authoritative planning boundary, without provider execution.
#[async_trait]
pub trait PlanningCatalog: Send + Sync {
    /// Checks target management, source use, and exact external rights before facts.
    ///
    /// Implementations must derive current publication, schemas, image and policy
    /// from trusted storage; configured platform policy is never request input.
    ///
    /// # Errors
    /// Rejects authorization loss, unavailable resources or invalid source facts.
    async fn load_for_plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PlanningRequest,
    ) -> Result<PlanningCatalogSnapshot, DeploymentError>;
}

/// Preview intent and observations; conveys no admission receipt or authority.
#[derive(Debug, Clone, Serialize)]
pub struct RecipePlan {
    intent: DeploymentIntent,
    sources: Vec<SourceObservation>,
}

impl RecipePlan {
    /// Returns immutable v1 intent, which must be admitted independently.
    #[must_use]
    pub const fn intent(&self) -> &DeploymentIntent {
        &self.intent
    }
    /// Returns checked source observations separate from the frozen intent hash.
    #[must_use]
    pub fn sources(&self) -> &[SourceObservation] {
        &self.sources
    }
}

/// Resolves bounded requests only through an authenticated authoritative catalog.
pub struct RecipePlanner<C> {
    catalog: C,
}

impl<C: PlanningCatalog> RecipePlanner<C> {
    /// Creates a planner with its trusted catalog adapter.
    #[must_use]
    pub const fn new(catalog: C) -> Self {
        Self { catalog }
    }
    /// Loads current facts and freezes a matching declaration and resolved graph.
    ///
    /// # Errors
    /// Rejects missing or duplicate source observations and invalid resolution.
    pub async fn plan(
        &self,
        identity: &AuthenticatedIdentity,
        request: &PlanningRequest,
    ) -> Result<RecipePlan, DeploymentError> {
        let mut catalog = self.catalog.load_for_plan(identity, request).await?;
        let expected = request
            .declaration()
            .manifest()
            .resources
            .iter()
            .filter_map(|resource| match resource {
                ResourceDeclaration::Instance(instance) => Some((
                    instance.release.release_id.as_uuid(),
                    instance.release.release_agent_id.as_uuid(),
                )),
                ResourceDeclaration::Volume(_) => None,
            })
            .collect::<BTreeSet<_>>();
        let observed = catalog
            .sources
            .iter()
            .map(|source| {
                (
                    source.pin().release_id.as_uuid(),
                    source.pin().release_agent_id.as_uuid(),
                )
            })
            .collect::<BTreeSet<_>>();
        if expected != observed || catalog.sources.len() != observed.len() {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        let resolved = request.declaration().resolve(
            request.inputs(),
            &catalog.external,
            &catalog.releases,
        )?;
        let intent = DeploymentIntent::new(
            request.id(),
            request.project_id(),
            request.key().clone(),
            request.declaration(),
            &resolved,
        )?;
        catalog
            .sources
            .sort_by_key(|source| source.pin().release_agent_id.as_uuid());
        Ok(RecipePlan {
            intent,
            sources: catalog.sources,
        })
    }
}
