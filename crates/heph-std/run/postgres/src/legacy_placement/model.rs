use run_domain::{
    LegacyVmPlacement, LegacyVmPlacementConsumption, LegacyVmPlacementProducer,
    LegacyVmPlacementScope, RunKind, StartRun,
};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

use crate::errors::storage;

#[derive(sqlx::FromRow)]
struct PlacementRow {
    run_id: Uuid,
    command_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    project_id: Uuid,
    attachment_id: Option<Uuid>,
    run_kind: String,
    requires_state: bool,
    release_contract_hash: Vec<u8>,
    provider_namespace: Uuid,
    host_id: String,
    vm_id: String,
    producer_kind: String,
    consumption_kind: Option<String>,
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    run_id: runtime_types::RunId,
    expected: &LegacyVmPlacementScope,
) -> Result<LegacyVmPlacement, RepositoryError> {
    let row: PlacementRow = sqlx::query_as("SELECT plan.* FROM run_legacy_vm_placements plan JOIN runs execution ON execution.id=plan.run_id WHERE plan.run_id=$1 AND legacy_vm_placement_matches(execution,plan)")
        .bind(run_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(storage)?
        .ok_or(RepositoryError::InvalidData("Legacy placement is held: positive exact birth proof missing"))?;
    let scope = LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(row.provider_namespace.to_string(), row.host_id)
            .map_err(storage)?,
    )
    .map_err(storage)?;
    if &scope != expected {
        return Err(RepositoryError::InvalidData(
            "Legacy placement is held: configured owner differs",
        ));
    }
    let kind = match row.run_kind.as_str() {
        "normal" => RunKind::Normal,
        "update" => RunKind::Update,
        "invocation" => {
            return Err(RepositoryError::InvalidData(
                "Invocation has no Legacy placement",
            ));
        }
        _ => return Err(RepositoryError::InvalidData("Legacy Run kind is invalid")),
    };
    let producer = match row.producer_kind.as_str() {
        "normal_request" => LegacyVmPlacementProducer::NormalRequest,
        "mailbox" => LegacyVmPlacementProducer::Mailbox,
        "update_hook" => LegacyVmPlacementProducer::UpdateHook,
        _ => return Err(RepositoryError::InvalidData("Legacy producer is invalid")),
    };
    let consumption = match row.consumption_kind.as_deref() {
        None => None,
        Some("provision") => Some(LegacyVmPlacementConsumption::Provision),
        Some("cleanup_only") => Some(LegacyVmPlacementConsumption::CleanupOnly),
        _ => {
            return Err(RepositoryError::InvalidData(
                "Legacy consumption is invalid",
            ));
        }
    };
    LegacyVmPlacement::new(
        StartRun {
            command_id: row.command_id.into(),
            run_id: row.run_id.into(),
            instance_id: row.instance_id.into(),
            instance_revision_id: row.instance_revision_id.into(),
            release_id: row.release_id.into(),
            release_agent_id: row.release_agent_id.into(),
            attachment_id: row.attachment_id.map(Into::into),
            kind,
            requires_state: row.requires_state,
        },
        row.project_id,
        row.release_contract_hash
            .try_into()
            .map_err(|_| RepositoryError::InvalidData("Legacy contract hash is invalid"))?,
        scope,
        VmId(row.vm_id),
        producer,
        consumption,
    )
    .map_err(storage)
}
