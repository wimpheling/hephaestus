use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{AgentUpdateId, ReleaseCommandKey};
use release_service::BeginUpdateHook;
use run_domain::{RunKind, StartRun};
use runtime_types::{AgentInstanceRevisionId, RunId};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn normal(pool: &PgPool) -> StartRun {
    let mut command = super::fixture::command();
    command.requires_state = false;
    super::support::seed_instance(pool, &command).await;
    command
}

pub async fn update(pool: &PgPool) -> (AuthenticatedIdentity, BeginUpdateHook) {
    let command = normal(pool).await;
    let candidate = AgentInstanceRevisionId::new();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable FROM agent_instance_revisions WHERE id=$2")
        .bind(candidate.as_uuid()).bind(command.instance_revision_id.as_uuid()).execute(pool).await.unwrap();
    let actor = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Plan fixture manager')")
        .bind(actor)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) SELECT project_id,$2 FROM agent_instances WHERE id=$1")
        .bind(command.instance_id.as_uuid()).bind(actor).execute(pool).await.unwrap();
    let update = AgentUpdateId::new();
    sqlx::query("INSERT INTO agent_updates(id,instance_id,expected_current_revision_id,candidate_revision_id,state,actor_id) VALUES($1,$2,$3,$4,'draining',$5)")
        .bind(update.as_uuid()).bind(command.instance_id.as_uuid()).bind(command.instance_revision_id.as_uuid()).bind(candidate.as_uuid()).bind(actor).execute(pool).await.unwrap();
    sqlx::query(
        "UPDATE agent_instances SET state='update_draining',run_gate_open=false WHERE id=$1",
    )
    .bind(command.instance_id.as_uuid())
    .execute(pool)
    .await
    .unwrap();
    // Test fixture principal only: production identity comes from middleware.
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(actor),
        "test-issuer",
        actor.to_string(),
        serde_json::json!({}),
        RequestId::new(),
    );
    (
        identity,
        BeginUpdateHook {
            command_key: ReleaseCommandKey::derive("plan-hook", &[update.as_uuid().as_bytes()]),
            update_id: update,
            hook_run_id: RunId::new(),
        },
    )
}

pub async fn load_hook(pool: &PgPool, id: RunId) -> StartRun {
    #[derive(sqlx::FromRow)]
    struct Hook {
        instance_id: Uuid,
        instance_revision_id: Uuid,
        release_id: Uuid,
        release_agent_id: Uuid,
        command_id: Uuid,
        requires_state: bool,
    }
    let row: Hook = sqlx::query_as("SELECT instance_id,instance_revision_id,release_id,release_agent_id,command_id,requires_state FROM runs WHERE id=$1")
        .bind(id.as_uuid()).fetch_one(pool).await.unwrap();
    StartRun {
        command_id: row.command_id.into(),
        run_id: id,
        instance_id: row.instance_id.into(),
        instance_revision_id: row.instance_revision_id.into(),
        release_id: row.release_id.into(),
        release_agent_id: row.release_agent_id.into(),
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: row.requires_state,
    }
}
