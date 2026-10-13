//! A real prospective request without any original Run/placement/consumption.

use super::{
    QualifiedOrdinaryRoutingDecision, QualifiedOrdinaryRoutingHoldReason as Hold,
    QualifiedOrdinaryUnborn, held, inbox, incompatible_producers, requests,
};
use crate::errors::storage;
use run_domain::StartRun;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct OutboxStart {
    id: Uuid,
    aggregate_type: String,
    aggregate_id: Uuid,
    subject: String,
    event_type: String,
    payload: Value,
}

pub(super) async fn inspect(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    supplied: Option<&StartRun>,
) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
    // No original Run was globally visible in this SAME RRRO snapshot. Reject
    // orphan/partial receipts and effect history even if a request also exists.
    if incompatible_producers(tx, run).await? || partial_lineage(tx, run).await? {
        return Ok(held(Hold::PartialUnbornLineage));
    }
    let rows = requests(
        tx,
        run,
        supplied.map(|command| command.command_id.as_uuid()),
    )
    .await?;
    let [request] = rows.as_slice() else {
        return Ok(held(if rows.is_empty() {
            Hold::MissingRequest
        } else {
            Hold::ContradictoryOriginal
        }));
    };
    let start = match request.compare(supplied) {
        Ok(start) => start,
        Err(reason) => return Ok(held(reason)),
    };
    if start.run_id != run {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    if command_lineage(tx, &start).await? {
        return Ok(held(Hold::PartialUnbornLineage));
    }
    if let Some(inbox) = inbox(tx, &start).await? {
        if !inbox.matches(&start)? {
            return Ok(held(Hold::ContradictoryOriginal));
        }
        if inbox.processed {
            return Ok(held(Hold::PartialUnbornLineage));
        }
    }
    let commands: Vec<OutboxStart> = sqlx::query_as(
        "SELECT id,aggregate_type,aggregate_id,subject,event_type,payload
        FROM public.outbox WHERE (aggregate_id=$1 OR payload->>'run_id'=$1::text OR payload->>'command_id'=$2)
        AND (subject='hephaestus.run.start' OR event_type='run.start.v1') LIMIT 2",
    )
    .bind(run.as_uuid())
    .bind(start.command_id.to_string())
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let [command] = commands.as_slice() else {
        return Ok(held(Hold::ContradictoryOriginal));
    };
    if !outbox_matches(command, &start)? {
        return Ok(held(Hold::ContradictoryOriginal));
    }
    Ok(QualifiedOrdinaryRoutingDecision::ProspectivelyUnborn(
        Box::new(QualifiedOrdinaryUnborn {
            start,
            source: request.source(),
        }),
    ))
}

async fn partial_lineage(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
) -> Result<bool, RepositoryError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.run_legacy_vm_placements WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_events WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.agent_instance_volume_leases WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_cleanup_targets WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_authorization_snapshots WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.runtime_authority_sessions WHERE run_id=$1)",
    )
    .bind(run.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)
}

async fn command_lineage(
    tx: &mut Transaction<'_, Postgres>,
    start: &StartRun,
) -> Result<bool, RepositoryError> {
    // A command already attached to another original cannot be treated as unborn.
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.runs WHERE command_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_legacy_vm_placements WHERE command_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_vm_plan_admissions WHERE command_id=$1)
        OR EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE start_command_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_invocation_consumptions WHERE start_command_id=$1)
        OR EXISTS(SELECT 1 FROM public.mailbox_delivery_attempts WHERE command_id=$1)")
        .bind(start.command_id.as_uuid()).fetch_one(&mut **tx).await.map_err(storage)
}

fn outbox_matches(command: &OutboxStart, start: &StartRun) -> Result<bool, RepositoryError> {
    if command.id.is_nil()
        || command.aggregate_type != "forge"
        || command.aggregate_id != start.run_id.as_uuid()
        || command.subject != "hephaestus.run.start"
        || command.event_type != "run.start.v1"
    {
        return Ok(false);
    }
    let Value::Object(mut payload) = command.payload.clone() else {
        return Ok(false);
    };
    // Exactly the finite metadata appended by the actual Forge outbox producer;
    // reject unknown extras instead of permissive DTO deserialization.
    if payload.remove("schema_version") != Some(json!(1))
        || payload.remove("message_id") != Some(json!(command.id))
        || payload.remove("idempotency_key") != Some(json!(command.id))
        || payload.remove("request_id") != Some(Value::Null)
        || payload.remove("trace_id") != Some(Value::Null)
    {
        return Ok(false);
    }
    Ok(Value::Object(payload) == serde_json::to_value(start).map_err(storage)?)
}
