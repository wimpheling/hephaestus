use super::{RetirementContext, instance_client, mutation_context, opaque, release_client};
use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::{ReleaseCommandKey, ReleaseId};
use release_postgres::ReleaseService;
use rpc_proto::messages::hephaestus::{
    common::v1::{NetworkPolicy, RuntimePolicy},
    instance::v1::ImportAgentRequest,
    release::v1::{GetReleaseRequest, ReleaseState},
};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) async fn retire_release(
    ctx: &RetirementContext<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Release retirement uses the existing authenticated domain command. The
    // command is deliberately outside the RPC surface; history is inspected
    // through GetRelease and a new ImportAgent RPC must be rejected.
    let release_id = ReleaseId::from_uuid(ctx.instance.release);
    let release_api = release_client(ctx, "/hephaestus.release.v1.ReleaseService/GetRelease")?;
    let before_release = release_api
        .get_release(GetReleaseRequest {
            release_id: opaque(ctx.instance.release).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or("published release missing")?;
    assert_eq!(
        before_release.state.to_i32(),
        ReleaseState::Published as i32
    );
    let source_commit = before_release.source_commit.clone();
    let release_service =
        ReleaseService::new(ctx.pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let revoke_key =
        ReleaseCommandKey::derive("cooking-retire-release", &[ctx.instance.release.as_bytes()]);
    release_service
        .revoke(ctx.owner, revoke_key, release_id)
        .await?;
    // The same production command key is safe to replay after a transport
    // retry and must not append a second logical retirement.
    release_service
        .revoke(ctx.owner, revoke_key, release_id)
        .await?;
    let outsider_key = ReleaseCommandKey::derive(
        "cooking-outsider-retire-release",
        &[ctx.instance.release.as_bytes()],
    );
    let outsider_error = release_service
        .revoke(ctx.outsider, outsider_key, release_id)
        .await
        .expect_err("outsider release revoke unexpectedly authorized");
    assert!(matches!(
        outsider_error,
        release_postgres::ReleaseServiceError::AuthorizationDenied
    ));
    let retired_release = release_api
        .get_release(GetReleaseRequest {
            release_id: opaque(ctx.instance.release).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or("revoked release history missing")?;
    assert_eq!(retired_release.state.to_i32(), ReleaseState::Revoked as i32);
    assert_eq!(retired_release.source_commit, source_commit);

    let instance_count_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_instances WHERE project_id = $1")
            .bind(ctx.project_id)
            .fetch_one(ctx.pool)
            .await?;
    let import_api = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )?;
    let import = import_api
        .import_agent(ImportAgentRequest {
            context: mutation_context("cooking-revoked-release-import").into(),
            project_id: opaque(ctx.project_id).into(),
            release_agent_id: opaque(ctx.instance.release_agent).into(),
            name: format!("cooking-revoked-{}", Uuid::new_v4().simple()),
            parameters: ctx.import_parameters.clone(),
            selected_policy: RuntimePolicy {
                vcpus: 1,
                memory_mib: 256,
                network: NetworkPolicy::BrokerOnly.into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await;
    let import_error = import.expect_err("ImportAgent accepted a revoked release");
    assert_eq!(
        import_error.code,
        connectrpc::error::ErrorCode::FailedPrecondition
    );
    assert_eq!(
        import_error.message.as_deref(),
        Some("operation precondition failed")
    );
    let instance_count_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_instances WHERE project_id = $1")
            .bind(ctx.project_id)
            .fetch_one(ctx.pool)
            .await?;
    assert_eq!(instance_count_after, instance_count_before);
    Ok(())
}
