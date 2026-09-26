use super::{ProvenanceBaseline, RetirementContext, opaque, run_client};

pub(crate) async fn effect_counts(
    ctx: &RetirementContext<'_>,
) -> Result<(i64, i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
            (SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1),
            (SELECT count(*) FROM runs WHERE instance_id = $2)",
    )
    .bind(ctx.mailbox_id)
    .bind(ctx.instance.instance)
    .fetch_one(ctx.pool)
    .await
}

pub(crate) async fn assert_final_provenance(
    ctx: &RetirementContext<'_>,
    baseline: &ProvenanceBaseline,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let provenance = run_client(ctx, "/hephaestus.run.v1.RunService/GetRunProvenance")?
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
            .ok_or("final provenance run ID missing")?
            .value,
        ctx.retained_run_id.to_string()
    );
    assert_eq!(
        provenance
            .authorization_snapshot_id
            .as_option()
            .ok_or("final authorization snapshot missing")?
            .value,
        baseline.snapshot_id
    );
    assert_eq!(
        provenance.authorization_model_version,
        baseline.model_version
    );
    assert_eq!(
        provenance.authorization_snapshot_hash,
        baseline.snapshot_hash
    );
    let final_use_ids = provenance
        .https_uses
        .iter()
        .filter_map(|usage| usage.id.as_option().map(|id| id.value.clone()))
        .collect::<Vec<_>>();
    assert_eq!(final_use_ids, baseline.use_ids);
    Ok(())
}
