#![allow(unused_imports)]
use super::denial::{accepted_receive_count, canonical_main_ref};
use super::git_mutations::{append_human_and_push, initialize_session_checkout};
use super::rpc_helpers::{git_output, instance_client, mutation_context, opaque, response_id};
use super::secrets::seed_model_secret;
use super::*;
use super::{DENIAL_HUMAN_RECORD_ID, HUMAN_RECORD_ID, MODEL_RULE};
use crate::golden_modules::cooking_builds::PublishedCookingRepository;
use forge_domain::{GitRef, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::{
    common::v1::{ParameterValue, RuntimePolicy, parameter_value::Value},
    instance::v1::{
        CapabilityBindingSelection, CreateAttachmentRequest, ImportAgentRequest, RefSelector,
        ReviseCapabilitiesRequest, TriggerPolicy, ref_selector,
    },
};
use sqlx::PgPool;
use std::{path::Path, time::Duration};
use uuid::Uuid;

pub(crate) struct NonBrowserState {
    pub(crate) built: PublishedCookingRepository,
    pub(crate) repository_id: Uuid,
    pub(crate) instance_id: Uuid,
    pub(crate) revision_id: Uuid,
    pub(crate) attachment_id: Uuid,
    pub(crate) run_id: Uuid,
    pub(crate) human_commit: String,
    pub(crate) denial_probe: bool,
    pub(crate) denial_source_repository_id: Option<Uuid>,
    pub(crate) denial_other_repository_id: Option<Uuid>,
    pub(crate) accepted_before_turn: i64,
    pub(crate) denial_source_ref_before: Option<String>,
    pub(crate) denial_other_ref_before: Option<String>,
    pub(crate) denial_source_accepts_before: Option<i64>,
    pub(crate) denial_other_accepts_before: Option<i64>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// Setup preserves the exact production fixture sequence and independent handles.
pub(crate) async fn prepare_nonbrowser(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    organization: forge_domain::OrganizationId,
    repositories: &PgForgeRepository,
    identity: &AuthenticatedIdentity,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    _timeout: Duration,
    built: PublishedCookingRepository,
    denial_probe: bool,
    denial_source_repository_id: Option<Uuid>,
    denial_other_repository_id: Option<Uuid>,
) -> NonBrowserState {
    let session_repository = repositories
        .create_repository_trusted(&CreateRepository {
            project_id: project,
            name: format!("session-chat-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main").expect("session main ref"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("create session repository");
    let (checkout, baseline) = {
        let checkout = root.join("session-chat-checkout");
        initialize_session_checkout(
            &checkout,
            source_root,
            session_repository.id.as_uuid(),
            git_token,
            running,
            &format!("user:{}", identity.user_id),
        )
        .await;
        let baseline = git_output(&checkout, &["rev-parse", "HEAD"]).await;
        (Some(checkout), Some(baseline))
    };
    let mut parameters = vec![ParameterValue {
        name: String::from("model_rule_id"),
        value: Some(Value::StringValue(MODEL_RULE.to_string())),
        ..Default::default()
    }];
    if denial_probe {
        parameters.extend([
            ParameterValue {
                name: String::from("denial_source_repository_id"),
                value: Some(Value::StringValue(
                    denial_source_repository_id
                        .expect("denial source repository ID")
                        .to_string(),
                )),
                ..Default::default()
            },
            ParameterValue {
                name: String::from("denial_other_repository_id"),
                value: Some(Value::StringValue(
                    denial_other_repository_id
                        .expect("denial comparison repository ID")
                        .to_string(),
                )),
                ..Default::default()
            },
        ]);
    }

    let imported = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )
    .expect("session ImportAgent client")
    .import_agent(ImportAgentRequest {
        context: mutation_context("session-chat-import").into(),
        project_id: opaque(project.as_uuid()).into(),
        release_agent_id: opaque(built.release_agent_id).into(),
        name: String::from("reference-session-chat"),
        parameters,
        selected_policy: RuntimePolicy {
            vcpus: 1,
            memory_mib: 256,
            network: rpc_proto::messages::hephaestus::common::v1::NetworkPolicy::BrokerOnly.into(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
    .await
    .expect("ImportAgent session-chat release")
    .into_owned();
    let instance_id = response_id(imported.instance_id.into_option(), "session instance")
        .expect("session instance ID");
    let revision_id = response_id(imported.revision_id.into_option(), "session revision")
        .expect("session revision ID");
    let attachment = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
    )
    .expect("session CreateAttachment client")
    .create_attachment(CreateAttachmentRequest {
        context: mutation_context("session-chat-attachment").into(),
        instance_id: opaque(instance_id).into(),
        repository_id: opaque(session_repository.id.as_uuid()).into(),
        ref_selector: RefSelector {
            selector: Some(ref_selector::Selector::Exact(String::from(
                "refs/heads/main",
            ))),
            ..Default::default()
        }
        .into(),
        trigger_policy: TriggerPolicy::Push.into(),
        ..Default::default()
    })
    .await
    .expect("CreateAttachment session-chat repository")
    .into_owned();
    let attachment_id = response_id(attachment.attachment_id.into_option(), "session attachment")
        .expect("session attachment ID");

    let revised = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities",
    )
    .expect("session ReviseCapabilities client")
    .revise_capabilities(ReviseCapabilitiesRequest {
        context: mutation_context("session-chat-capabilities").into(),
        instance_id: opaque(instance_id).into(),
        expected_revision_id: opaque(revision_id).into(),
        bindings: vec![CapabilityBindingSelection {
            slot_key: String::from("session"),
            resource_kind: String::from("repository"),
            resource_id: opaque(session_repository.id.as_uuid()).into(),
            granted_operations: vec![String::from("git_read"), String::from("update_ref")],
            ..Default::default()
        }],
        ..Default::default()
    })
    .await
    .expect("revise session capability")
    .into_owned();
    assert!(
        !revised.runnable,
        "the capability revision must remain gated on the required secret"
    );
    let capability_revision_id = response_id(
        revised.instance_revision_id.into_option(),
        "session capability revision",
    )
    .expect("session capability revision ID");
    let _active_revision_id = seed_model_secret(
        pool,
        organization,
        project,
        identity,
        instance_id,
        capability_revision_id,
        attachment_id,
    )
    .await;

    let user_id = identity.user_id.to_string();
    let checkout = checkout.expect("standalone session checkout");
    let baseline = baseline.expect("standalone session baseline");
    let accepted_before_turn = accepted_receive_count(pool, session_repository.id.as_uuid()).await;
    let denial_source_ref_before = match denial_source_repository_id {
        Some(repository_id) => canonical_main_ref(pool, repository_id).await,
        None => None,
    };
    let denial_other_ref_before = match denial_other_repository_id {
        Some(repository_id) => canonical_main_ref(pool, repository_id).await,
        None => None,
    };
    let denial_source_accepts_before = denial_source_repository_id
        .map(|repository_id| accepted_receive_count(pool, repository_id));
    let denial_source_accepts_before = match denial_source_accepts_before {
        Some(count) => Some(count.await),
        None => None,
    };
    let denial_other_accepts_before =
        denial_other_repository_id.map(|repository_id| accepted_receive_count(pool, repository_id));
    let denial_other_accepts_before = match denial_other_accepts_before {
        Some(count) => Some(count.await),
        None => None,
    };
    append_human_and_push(
        &checkout,
        source_root,
        git_token,
        running,
        &user_id,
        baseline.as_str(),
        if denial_probe {
            DENIAL_HUMAN_RECORD_ID
        } else {
            HUMAN_RECORD_ID
        },
    )
    .await;
    let human_commit = git_output(&checkout, &["rev-parse", "HEAD"]).await;
    let run_id: Uuid = sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $4
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $3
          LIMIT 1",
    )
    .bind(session_repository.id.as_uuid())
    .bind(instance_id)
    .bind(&human_commit)
    .bind(attachment_id)
    .fetch_one(pool)
    .await
    .expect("session-chat push run request");
    NonBrowserState {
        built,
        repository_id: session_repository.id.as_uuid(),
        instance_id,
        revision_id,
        attachment_id,
        run_id,
        human_commit,
        denial_probe,
        denial_source_repository_id,
        denial_other_repository_id,
        accepted_before_turn,
        denial_source_ref_before,
        denial_other_ref_before,
        denial_source_accepts_before,
        denial_other_accepts_before,
    }
}
