use recipe_application::{
    DeploymentError, InstanceSafetyInventory, PreparedInstanceSafetyObservation,
    PreparedInstanceSafetyState, PreparedInstanceSafetyTarget,
};
use release_domain::InstanceRemovalId;
use sqlx::types::Json;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct Row {
    pub birth_proven: bool,
    pub removal_id: Option<Uuid>,
    pub cleanup_ready: bool,
    pub inventory: Json<InstanceSafetyInventory>,
}
pub fn observation(
    row: Row,
    target: &PreparedInstanceSafetyTarget,
) -> Result<PreparedInstanceSafetyObservation, DeploymentError> {
    if row.cleanup_ready && (!row.birth_proven || row.removal_id.is_none()) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let state = if !row.birth_proven {
        PreparedInstanceSafetyState::Unproven
    } else if row.removal_id.is_none() {
        PreparedInstanceSafetyState::Created
    } else if row.cleanup_ready {
        PreparedInstanceSafetyState::Drained
    } else {
        PreparedInstanceSafetyState::ClosedHeld
    };
    let inventory = row.inventory.0;
    unique(
        &inventory
            .runs
            .iter()
            .map(|id| id.as_uuid())
            .collect::<Vec<_>>(),
    )?;
    unique(&inventory.leases)?;
    unique(&inventory.mount_grants)?;
    unique(
        &inventory
            .volumes
            .iter()
            .map(|id| id.as_uuid())
            .collect::<Vec<_>>(),
    )?;
    unique(
        &inventory
            .consumers
            .iter()
            .map(|id| id.as_uuid())
            .collect::<Vec<_>>(),
    )?;
    if inventory.work.iter().any(|work| work.id.is_nil())
        || inventory
            .work
            .iter()
            .map(|work| (work.kind, work.id))
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != inventory.work.len()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(PreparedInstanceSafetyObservation {
        target: target.clone(),
        state,
        removal_id: row.removal_id.map(InstanceRemovalId::from_uuid),
        inventory,
    })
}
fn unique(ids: &[Uuid]) -> Result<(), DeploymentError> {
    if ids.iter().any(Uuid::is_nil)
        || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
