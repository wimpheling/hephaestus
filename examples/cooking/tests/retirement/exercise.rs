use super::{
    ProvenanceBaseline, RetirementContext, assert_final_provenance, instance_client,
    mutation_context, opaque, remove_attachment_and_assert, retire_gateway, retire_release,
    run_client, wait_for_control_terminal, wait_for_failed_retry,
};
use rpc_proto::messages::hephaestus::{
    instance::v1::{GetInstanceRequest, RemovalState, RemoveAttachmentRequest},
    run::v1::{
        GetRunRequest, RequestControlRequest, RunControlKind, RunControlTarget, run_control_target,
    },
};
use uuid::Uuid;

pub async fn exercise(
    ctx: &RetirementContext<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let provenance = inspect_run_and_attachment(ctx).await?;
    verify_removed_attachment_retry(ctx).await?;
    retire_gateway(ctx).await?;
    retire_release(ctx).await?;
    assert_final_provenance(ctx, &provenance).await?;
    Ok(())
}

pub(crate) async fn inspect_run_and_attachment(
    ctx: &RetirementContext<'_>,
) -> Result<ProvenanceBaseline, Box<dyn std::error::Error + Send + Sync>> {
    // Capture authorized history before retirement, then require the same
    // exact run and provenance chain after each resource transition.
    let run_api = run_client(ctx, "/hephaestus.run.v1.RunService/GetRun")?;
    let before = run_api
        .get_run(GetRunRequest {
            run_id: opaque(ctx.retained_run_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let before_run = before.run.into_option().ok_or("retained run missing")?;
    assert_eq!(
        before_run.id.into_option().ok_or("run ID missing")?.value,
        ctx.retained_run_id.to_string()
    );

    let provenance_client = run_client(ctx, "/hephaestus.run.v1.RunService/GetRunProvenance")?;
    let provenance = provenance_client
        .get_run_provenance(
            rpc_proto::messages::hephaestus::run::v1::GetRunProvenanceRequest {
                run_id: opaque(ctx.retained_run_id).into(),
                ..Default::default()
            },
        )
        .await?
        .into_owned();
    assert_eq!(
        provenance
            .run_id
            .into_option()
            .ok_or("provenance run ID missing")?
            .value,
        ctx.retained_run_id.to_string()
    );
    assert!(provenance.authorization_snapshot_id.is_set());
    let baseline = ProvenanceBaseline {
        snapshot_id: provenance
            .authorization_snapshot_id
            .as_option()
            .ok_or("authorization snapshot ID missing")?
            .value
            .clone(),
        model_version: provenance.authorization_model_version.clone(),
        snapshot_hash: provenance.authorization_snapshot_hash.clone(),
        use_ids: provenance
            .https_uses
            .iter()
            .filter_map(|usage| usage.id.as_option().map(|id| id.value.clone()))
            .collect(),
    };

    // Tombstone the exact attachment through the mediated instance RPC.
    let instance_api = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/RemoveAttachment",
    )?;
    let remove = RemoveAttachmentRequest {
        context: mutation_context("cooking-retire-attachment").into(),
        attachment_id: opaque(ctx.instance.attachment).into(),
        ..Default::default()
    };
    let first = instance_api
        .remove_attachment(remove.clone())
        .await?
        .into_owned();
    assert_eq!(first.state.to_i32(), RemovalState::Removed as i32);
    assert!(first.receipt.is_set());
    let replay = instance_api.remove_attachment(remove).await?.into_owned();
    assert_eq!(replay.state.to_i32(), RemovalState::Removed as i32);
    assert_eq!(replay.receipt, first.receipt);

    let instance_read = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
    )?;
    let instance = instance_read
        .get_instance(GetInstanceRequest {
            instance_id: opaque(ctx.instance.instance).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .instance
        .into_option()
        .ok_or("retained instance missing")?;
    let attachment = instance
        .attachments
        .iter()
        .find(|attachment| {
            attachment
                .id
                .as_option()
                .is_some_and(|id| id.value == ctx.instance.attachment.to_string())
        })
        .ok_or("removed attachment history missing")?;
    assert!(!attachment.enabled);
    assert!(attachment.removed_at.is_set());
    Ok(baseline)
}

pub(crate) async fn verify_removed_attachment_retry(
    ctx: &RetirementContext<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The cooking run is mailbox-originated and has no run_requests row. The
    // retry security proof therefore uses the separately seeded, completed
    // forge run whose request owns the dedicated instance and attachment.
    remove_attachment_and_assert(ctx, ctx.retry_instance, "cooking-retire-forge-attachment")
        .await?;

    // A completed run retry may be accepted into the durable control queue,
    // but the removed attachment must deny launch before any guest is built.
    // This permits the existing control-plane retry contract while checking
    // the security boundary at the runtime launch authorizer.
    let existing_retry_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM run_requests WHERE retry_of_run_id = $1")
            .bind(ctx.retry_source_run_id)
            .fetch_all(ctx.pool)
            .await?;
    let retry_client = run_client(ctx, "/hephaestus.run.v1.RunService/RequestControl")?;
    let retry = retry_client
        .request_control(RequestControlRequest {
            context: mutation_context("cooking-retired-attachment-retry").into(),
            kind: RunControlKind::Retry.into(),
            repository_id: opaque(ctx.retry_repository_id).into(),
            target: RunControlTarget {
                target: Some(run_control_target::Target::RunId(Box::new(opaque(
                    ctx.retry_source_run_id,
                )))),
                ..Default::default()
            }
            .into(),
            reason: String::from("retired attachment must deny guest launch"),
            ..Default::default()
        })
        .await;
    match retry {
        Err(error) => assert!(matches!(
            error.code,
            connectrpc::error::ErrorCode::FailedPrecondition
                | connectrpc::error::ErrorCode::NotFound
        )),
        Ok(response) => {
            let control_id = response
                .into_owned()
                .control_request_id
                .into_option()
                .ok_or("retry control ID missing")?
                .value
                .parse::<Uuid>()?;
            let state = wait_for_control_terminal(ctx.pool, control_id).await?;
            assert!(state == "completed" || state == "failed");
            if state == "completed" {
                wait_for_failed_retry(ctx.pool, ctx.retry_source_run_id, &existing_retry_ids)
                    .await?;
            } else {
                let diagnostics: serde_json::Value =
                    sqlx::query_scalar("SELECT diagnostics FROM control_requests WHERE id = $1")
                        .bind(control_id)
                        .fetch_one(ctx.pool)
                        .await?;
                assert_eq!(
                    diagnostics,
                    serde_json::json!([{"code": "authorization_denied"}]),
                    "an unrelated control failure does not prove retirement denial"
                );
            }
        }
    }
    let after = run_client(ctx, "/hephaestus.run.v1.RunService/GetRun")?
        .get_run(GetRunRequest {
            run_id: opaque(ctx.retry_source_run_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    assert_eq!(
        after
            .run
            .into_option()
            .ok_or("forge retry source run disappeared")?
            .id
            .into_option()
            .ok_or("run ID missing")?
            .value,
        ctx.retry_source_run_id.to_string()
    );
    Ok(())
}
