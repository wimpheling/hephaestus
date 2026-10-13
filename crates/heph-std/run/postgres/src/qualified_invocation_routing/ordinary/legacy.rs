//! Existing worker-granted immutable Legacy helper, never a live producer predicate.

use super::{
    OriginalRun, QualifiedOrdinaryEvidence, QualifiedOrdinaryRoute,
    QualifiedOrdinaryRoutingDecision, QualifiedOrdinaryRoutingHoldReason as Hold, held, inbox,
    incompatible_producers, requests,
};
use crate::{errors::storage, model::parse_state};
use release_service::InstanceExecutionConfiguration;
use run_domain::{LegacyVmPlacementScope, StartRun};
use run_orchestrator::RepositoryError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

#[derive(sqlx::FromRow)]
struct Placement {
    provider_namespace: Uuid,
    host_id: String,
    vm_id: String,
    project_id: Uuid,
    release_contract_hash: Vec<u8>,
    producer_kind: String,
    producer_id: Uuid,
    actor_id: Option<Uuid>,
    request_id: Option<Uuid>,
    consumed: bool,
    consumption_kind: Option<String>,
    exact: bool,
}

pub(super) async fn inspect(
    tx: &mut Transaction<'_, Postgres>,
    expected: &InstanceExecutionConfiguration,
    original: OriginalRun,
    supplied: Option<&StartRun>,
) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
    let plans:Vec<Placement>=sqlx::query_as("SELECT plan.provider_namespace,plan.host_id,plan.vm_id,
        plan.project_id,plan.release_contract_hash,plan.producer_kind,plan.producer_id,plan.actor_id,
        plan.request_id,plan.consumed_at IS NOT NULL AS consumed,plan.consumption_kind,
        public.legacy_vm_placement_matches(execution,plan) IS TRUE AS exact
        FROM public.run_legacy_vm_placements plan JOIN public.runs execution ON execution.id=plan.run_id
        WHERE plan.run_id=$1 LIMIT 2")
        .bind(original.id).fetch_all(&mut **tx).await.map_err(storage)?;
    let [plan] = plans.as_slice() else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    if !plan.exact
        || !original.legacy_vm_placement_required
        || original.vm_plan_admission_required
        || original.vm_provider_namespace.is_some()
        || original.vm_host_id.is_some()
        || original.vm_binding_kind.is_some()
    {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    if plan.producer_kind != "normal_request" {
        return Ok(held(Hold::UnsupportedProducer));
    }
    let Ok(owner) =
        VmProviderOwnerScope::new(plan.provider_namespace.to_string(), plan.host_id.clone())
    else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    if LegacyVmPlacementScope::new(owner.clone()).is_err() {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    if &owner != expected.provider_scope() {
        return Ok(held(Hold::ForeignOwner));
    }
    let Ok(start) = original.start() else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    let rows = requests(tx, start.run_id, Some(start.command_id.as_uuid())).await?;
    let [request] = rows.as_slice() else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    let request_start = match request.compare(supplied) {
        Ok(start) => start,
        Err(reason) => return Ok(held(reason)),
    };
    // Placement producer identity is the immutable run_requests row ID.
    let producer_request_id = request.id;
    if request_start != start
        || plan.producer_id != producer_request_id
        || plan.actor_id != request.actor_id
        || plan.request_id != request.request_id
        || plan.project_id.is_nil()
        || plan.vm_id != start.run_id.to_string()
        || original
            .vm_id
            .as_ref()
            .is_some_and(|actual| actual != &plan.vm_id)
        || incompatible_producers(tx, start.run_id).await?
    {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    let Some(inbox) = inbox(tx, &start).await? else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    if !inbox.matches(&start)? {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    if !plan.consumed && plan.consumption_kind.is_none() {
        return Ok(held(Hold::UnconsumedOriginal));
    }
    // Normal birth consumes provision and processes Start in one committed TX.
    // Do not infer consumption from lifecycle state, empty history or UUID shape.
    if !plan.consumed || plan.consumption_kind.as_deref() != Some("provision") || !inbox.processed {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    let Ok(hash) = plan.release_contract_hash.as_slice().try_into() else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    let Ok(run_state) = parse_state(&original.state) else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    Ok(QualifiedOrdinaryRoutingDecision::Qualified(Box::new(
        QualifiedOrdinaryRoute {
            original_start: start,
            source: request.source(),
            owner,
            vm_id: VmId(plan.vm_id.clone()),
            project_id: plan.project_id,
            release_contract_hash: hash,
            evidence: QualifiedOrdinaryEvidence::LegacyProvisionConsumed,
            run_state,
            cancellation_requested: original.cancellation_requested,
            acquisition_closed: original.acquisition_closed,
        },
    )))
}
