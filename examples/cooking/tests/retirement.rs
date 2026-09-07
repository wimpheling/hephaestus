//! Authenticated retirement checks for the cooking acceptance path.
//!
//! Resource changes use generated RPCs or the existing authenticated release
//! service command. SQL is limited to read-only effect and durable-history
//! inspection.

use authz_postgres::PostgresMelangeAuthorizer;
use connectrpc::client::{ClientConfig, HttpClient};
use identity_domain::AuthenticatedIdentity;
use release_domain::{ReleaseCommandKey, ReleaseId};
use release_postgres::ReleaseService;
use rpc_proto::{
    connect::hephaestus::{
        gateway::v1::GatewayServiceClient, instance::v1::AgentInstanceServiceClient,
        release::v1::ReleaseServiceClient, run::v1::RunServiceClient,
    },
    messages::hephaestus::{
        common::v1::{NetworkPolicy, OpaqueId, RequestContext, RuntimePolicy},
        gateway::v1::{GatewayLifecycle, GetGatewayRequest, SetGatewayLifecycleRequest},
        instance::v1::{
            GetInstanceRequest, ImportAgentRequest, RemovalState, RemoveAttachmentRequest,
        },
        release::v1::{GetReleaseRequest, ReleaseState},
        run::v1::{
            GetRunRequest, RequestControlRequest, RunControlKind, RunControlTarget,
            run_control_target,
        },
    },
};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

use super::SeededInstance;

pub struct RetirementContext<'a> {
    pub pool: &'a PgPool,
    pub running: &'a hephaestus_app::RunningHephaestus,
    pub token_factory: &'a (dyn Fn(&str) -> String + Send + Sync),
    pub owner: &'a AuthenticatedIdentity,
    pub outsider: &'a AuthenticatedIdentity,
    pub instance: &'a SeededInstance,
    pub retained_run_id: Uuid,
    pub retry_instance: &'a SeededInstance,
    pub retry_source_run_id: Uuid,
    pub retry_repository_id: Uuid,
    pub gateway_id: Uuid,
    pub project_id: Uuid,
    pub mailbox_id: Uuid,
    pub public_url: &'a str,
    pub valid_inbound_credential: &'a str,
    pub import_parameters: Vec<rpc_proto::messages::hephaestus::common::v1::ParameterValue>,
}

struct ProvenanceBaseline {
    snapshot_id: String,
    model_version: String,
    snapshot_hash: String,
    use_ids: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct RetryObservation {
    id: Uuid,
    state: String,
    outcome: Option<String>,
    failure: Option<String>,
    vm_id: Option<String>,
}

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

async fn inspect_run_and_attachment(
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

async fn verify_removed_attachment_retry(
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

async fn remove_attachment_and_assert(
    ctx: &RetirementContext<'_>,
    instance: &SeededInstance,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let instance_api = instance_client(
        ctx,
        "/hephaestus.instance.v1.AgentInstanceService/RemoveAttachment",
    )?;
    let remove = RemoveAttachmentRequest {
        context: mutation_context(operation).into(),
        attachment_id: opaque(instance.attachment).into(),
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
    )?
    .get_instance(GetInstanceRequest {
        instance_id: opaque(instance.instance).into(),
        ..Default::default()
    })
    .await?
    .into_owned()
    .instance
    .into_option()
    .ok_or("retired attachment instance missing")?;
    let attachment = instance_read
        .attachments
        .iter()
        .find(|attachment| {
            attachment
                .id
                .as_option()
                .is_some_and(|id| id.value == instance.attachment.to_string())
        })
        .ok_or("removed attachment history missing")?;
    assert!(!attachment.enabled);
    assert!(attachment.removed_at.is_set());
    Ok(())
}

async fn retire_gateway(
    ctx: &RetirementContext<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Remove the gateway through lifecycle CAS. `removed` is the supported
    // tombstone state; the edge's enabled-route query then yields an exact 404.
    let gateway_api = gateway_client(ctx, "/hephaestus.gateway.v1.GatewayService/GetGateway")?;
    let before_gateway = gateway_api
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(ctx.gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let old_revision = before_gateway
        .gateway
        .as_option()
        .and_then(|g| g.active_revision_id.as_option())
        .ok_or("active gateway revision missing")?
        .value
        .clone();
    assert_eq!(
        before_gateway
            .gateway
            .as_option()
            .ok_or("gateway missing")?
            .lifecycle
            .to_i32(),
        GatewayLifecycle::Enabled as i32
    );

    let lifecycle_client = gateway_client(
        ctx,
        "/hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle",
    )?;
    let transition = SetGatewayLifecycleRequest {
        context: mutation_context("cooking-retire-gateway").into(),
        gateway_id: opaque(ctx.gateway_id).into(),
        expected: GatewayLifecycle::Enabled.into(),
        next: GatewayLifecycle::Removed.into(),
        ..Default::default()
    };
    let changed = lifecycle_client
        .set_gateway_lifecycle(transition.clone())
        .await?
        .into_owned();
    assert_eq!(
        changed
            .gateway
            .as_option()
            .ok_or("lifecycle response missing")?
            .lifecycle
            .to_i32(),
        GatewayLifecycle::Removed as i32
    );
    assert!(changed.receipt.is_set());
    // Lifecycle mutations are compare-and-swap operations: replaying the
    // original Enabled expectation after removal is a stale precondition.
    let replay_error = lifecycle_client
        .set_gateway_lifecycle(transition)
        .await
        .expect_err("replayed removed gateway CAS unexpectedly succeeded");
    assert_eq!(
        replay_error.code,
        connectrpc::error::ErrorCode::FailedPrecondition
    );
    assert_eq!(
        replay_error.message.as_deref(),
        Some("operation precondition failed")
    );

    let gateway_history = gateway_client(ctx, "/hephaestus.gateway.v1.GatewayService/GetGateway")?
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(ctx.gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let gateway = gateway_history
        .gateway
        .as_option()
        .ok_or("removed gateway history missing")?;
    assert_eq!(gateway.lifecycle.to_i32(), GatewayLifecycle::Removed as i32);
    assert_eq!(
        gateway
            .active_revision_id
            .as_option()
            .ok_or("active revision history missing")?
            .value,
        old_revision
    );
    assert!(!gateway_history.revisions.is_empty());

    let before_effects = effect_counts(ctx).await?;
    let response = reqwest::Client::new()
        .post(format!("{}/gateway/cooking/telegram", ctx.public_url.trim_end_matches('/')))
        .header("x-telegram-bot-api-secret-token", ctx.valid_inbound_credential)
        .json(&serde_json::json!({"update_id": 9002, "message": {"from": {"id": 1001}, "text": "retired-gateway"}}))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    response.bytes().await?;
    assert_eq!(effect_counts(ctx).await?, before_effects);
    Ok(())
}

async fn retire_release(
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

async fn wait_for_control_terminal(
    pool: &PgPool,
    id: Uuid,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state: String =
                sqlx::query_scalar("SELECT state FROM control_requests WHERE id = $1")
                    .bind(id)
                    .fetch_one(pool)
                    .await?;
            if state == "failed" || state == "completed" {
                return Ok::<String, sqlx::Error>(state);
            }
            assert!(state == "pending" || state == "processing");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??)
}

async fn wait_for_failed_retry(
    pool: &PgPool,
    source_run_id: Uuid,
    existing_retry_ids: &[Uuid],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let row: Option<RetryObservation> = sqlx::query_as(
                "SELECT run.id, run.state, run.outcome, run.failure, run.vm_id
                   FROM runs AS run
                   JOIN run_requests AS request ON request.run_id = run.id
                  WHERE request.retry_of_run_id = $1 AND run.id <> ALL($2)
                  ORDER BY run.created_at DESC, run.id DESC
                  LIMIT 1",
            )
            .bind(source_run_id)
            .bind(existing_retry_ids)
            .fetch_optional(pool)
            .await?;
            if let Some(retry) = row {
                if retry.state == "cleaned_up" {
                    assert_eq!(retry.outcome.as_deref(), Some("failed"));
                    assert!(
                        retry
                            .failure
                            .as_deref()
                            .is_some_and(|value| {
                                value
                                    == "invalid VM specification field \"instance_revision\": the exact revision or attachment is not runnable"
                            })
                    );
                    // The orchestrator may reserve the run ID as `vm_id` before
                    // validating the immutable spec; that reservation is not
                    // evidence that the provider created a VM.
                    let reserved_vm_id = retry.id.to_string();
                    assert!(
                        retry
                            .vm_id
                            .as_deref()
                            .is_none_or(|vm_id| vm_id == reserved_vm_id),
                        "denied retry has an unexpected VM identifier"
                    );
                    let lifecycle_events: i64 = sqlx::query_scalar(
                        "SELECT count(*)
                           FROM run_events
                          WHERE run_id = $1
                            AND event_type IN (
                                'vm.started', 'vm.ready', 'vm.exited',
                                'run.starting', 'run.running'
                            )",
                    )
                    .bind(retry.id)
                    .fetch_one(pool)
                    .await?;
                    assert_eq!(lifecycle_events, 0);
                    assert_ne!(retry.id, source_run_id);
                    return Ok::<(), sqlx::Error>(());
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;
    Ok(())
}

async fn effect_counts(ctx: &RetirementContext<'_>) -> Result<(i64, i64, i64), sqlx::Error> {
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

async fn assert_final_provenance(
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

fn mutation_context(key: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: key.into(),
        ..Default::default()
    }
}

fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

fn config(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    Ok(
        ClientConfig::new(format!("http://{}", ctx.running.http_addr()).parse()?)
            .with_default_header(
                http::header::AUTHORIZATION,
                http::HeaderValue::from_str(&format!("Bearer {}", (ctx.token_factory)(audience)))?,
            )
            .with_default_timeout(Duration::from_secs(30)),
    )
}

fn instance_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<AgentInstanceServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(AgentInstanceServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

fn gateway_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<GatewayServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(GatewayServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

fn release_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<ReleaseServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(ReleaseServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

fn run_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<RunServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(RunServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}
