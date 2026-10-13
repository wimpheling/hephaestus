mod records;

use release_service::InstanceExecutionConfiguration;
use run_domain::StartRun;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

use super::{
    QualifiedInvocationEvidence, QualifiedInvocationRoutingDecision,
    QualifiedInvocationRoutingHoldReason,
};
use crate::errors::storage;
use records::{Inbox, Invocation, OriginalRun};

pub(super) async fn inspect(
    transaction: &mut Transaction<'_, Postgres>,
    expected: &InstanceExecutionConfiguration,
    run: RunId,
    supplied: Option<&StartRun>,
) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
    if role != "hephaestus_worker" {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::WorkerRoleRequired,
        ));
    }
    inspect_in_snapshot(transaction, expected, run, supplied).await
}

// Caller retains the same role-checked worker RRRO snapshot and search path.
// This is comparison only; no projection authorizes an owning operation.
pub async fn inspect_in_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    expected: &InstanceExecutionConfiguration,
    run: RunId,
    supplied: Option<&StartRun>,
) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
    // Global Run lookup precedes profile/producer/configuration/owner checks.
    let original: Option<OriginalRun> = sqlx::query_as(
        "SELECT id,command_id,instance_id,instance_revision_id,release_id,
                release_agent_id,attachment_id,run_kind,requires_state,
                execution_profile,vm_id,vm_provider_namespace,vm_host_id,
                vm_binding_kind,state,cancel_requested_at,volume_acquisition_closed_at
         FROM public.runs WHERE id=$1",
    )
    .bind(run.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let Some(original) = original else {
        return Ok(held(QualifiedInvocationRoutingHoldReason::MissingRun));
    };
    if original.execution_profile != "owned_canonical" {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::UnsupportedProfile,
        ));
    }
    if original.run_kind != "invocation" {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::UnsupportedProducer,
        ));
    }
    inspect_invocation(transaction, expected, original, supplied).await
}

async fn inspect_invocation(
    transaction: &mut Transaction<'_, Postgres>,
    expected: &InstanceExecutionConfiguration,
    original: OriginalRun,
    supplied: Option<&StartRun>,
) -> Result<QualifiedInvocationRoutingDecision, RepositoryError> {
    let receipts: Vec<Invocation> = sqlx::query_as(
        "SELECT id,run_id,start_command_id,actor_id,request_id,command_key,
                input_hash,config_bytes,config_hash,config_json,admission_version
         FROM public.agent_instance_execution_invocations WHERE run_id=$1 LIMIT 2",
    )
    .bind(original.id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    let [receipt] = receipts.as_slice() else {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::ContradictoryOriginal,
        ));
    };
    let inbox: Option<Inbox> = sqlx::query_as(
        "SELECT command_type,payload,processed_at IS NOT NULL AS processed
         FROM public.command_inbox WHERE command_id=$1",
    )
    .bind(original.command_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let Some(inbox) = inbox else {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::ContradictoryOriginal,
        ));
    };
    let mut projection = match records::compare(&original, receipt, &inbox, expected, supplied)? {
        Ok(projection) => projection,
        Err(reason) => return Ok(held(reason)),
    };
    // Reuse the existing closed Invocation branch and full immutable graph.
    // Its ordinary producer branches are never used as lifecycle classifiers.
    let graph: bool = sqlx::query_scalar(
        "SELECT public.run_profile_producer_matches(execution) IS TRUE
         FROM public.runs execution WHERE execution.id=$1",
    )
    .bind(original.id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    if !graph {
        return Ok(held(
            QualifiedInvocationRoutingHoldReason::ContradictoryOriginal,
        ));
    }
    match consumption(transaction, original.id, inbox.processed).await? {
        Ok(evidence) => projection.evidence = evidence,
        Err(reason) => return Ok(held(reason)),
    }
    Ok(QualifiedInvocationRoutingDecision::Qualified(Box::new(
        projection,
    )))
}

async fn consumption(
    transaction: &mut Transaction<'_, Postgres>,
    run: uuid::Uuid,
    processed: bool,
) -> Result<
    Result<QualifiedInvocationEvidence, QualifiedInvocationRoutingHoldReason>,
    RepositoryError,
> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.run_invocation_consumptions WHERE run_id=$1",
    )
    .bind(run)
    .fetch_one(&mut **transaction)
    .await
    .map_err(storage)?;
    if count == 0 && !processed {
        return Ok(Ok(QualifiedInvocationEvidence::QueuedIntent115));
    }
    if count != 1 || !processed {
        return Ok(Err(
            QualifiedInvocationRoutingHoldReason::ContradictoryOriginal,
        ));
    }
    // Existing117 inspection compares all actual consumption/inbox/event pins
    // with this same immutable original graph, including exact config and owner.
    let consumed: bool =
        sqlx::query_scalar("SELECT public.inspect_qualified_invocation117($1) IS TRUE")
            .bind(run)
            .fetch_one(&mut **transaction)
            .await
            .map_err(storage)?;
    Ok(if consumed {
        Ok(QualifiedInvocationEvidence::Consumed117)
    } else {
        Err(QualifiedInvocationRoutingHoldReason::ContradictoryOriginal)
    })
}

const fn held(reason: QualifiedInvocationRoutingHoldReason) -> QualifiedInvocationRoutingDecision {
    QualifiedInvocationRoutingDecision::Held(reason)
}
