use super::browser_assertions::assert_browser_session;
use super::browser_setup::{
    SessionChatBrowserMode, grant_session_capability, run_session_chat_browser,
};
use super::browser_state::{BrowserRestartState, load_browser_restart_state};
use super::denial::prepare_denial_probe_source;
use super::git_mutations::initialize_session_checkout;
use super::model::SessionBrokerFixture;
use super::rpc_helpers::{git_output, instance_client, mutation_context, opaque};
use super::secrets::seed_model_import;
use super::*;
use super::{
    DENIAL_HUMAN_RECORD_ID, HUMAN_RECORD_ID, MODEL_RULE, concurrent_e2e_enabled,
    denial_probe_enabled, fork_e2e_enabled, restart_e2e_enabled,
};
use crate::golden_modules::cooking_builds::{CookingBuildContext, CookingIdentity};
use forge_domain::{GitRef, OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::{
    common::v1::{ParameterValue, RuntimePolicy, parameter_value::Value},
    instance::v1::ImportAgentRequest,
};
use sqlx::PgPool;
use std::{collections::HashSet, path::Path, time::Duration};
use uuid::Uuid;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
// This is the single end-to-end session acceptance phase and preserves its boundary inputs.
pub async fn exercise<'a>(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &'a Path,
    project: ProjectId,
    organization: OrganizationId,
    repositories: &PgForgeRepository,
    identity: &'a AuthenticatedIdentity,
    git_token: &'a str,
    rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    broker: SessionBrokerFixture,
    timeout: Duration,
) -> Option<BrowserRestartState<'a>> {
    let denial_probe = denial_probe_enabled();
    let browser_e2e =
        std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1");
    assert!(
        !concurrent_e2e_enabled() || (browser_e2e && restart_e2e_enabled()),
        "session-chat concurrency E2E requires browser and restart phases"
    );
    assert!(
        !fork_e2e_enabled() || (browser_e2e && restart_e2e_enabled() && concurrent_e2e_enabled()),
        "session-chat fork E2E requires browser, restart, and concurrency phases"
    );
    assert!(
        !denial_probe || !browser_e2e,
        "denial probe is a standalone session mode"
    );
    let (denial_other_repository_id, denial_source_root) = if denial_probe {
        let other = repositories
            .create_repository_trusted(&CreateRepository {
                project_id: project,
                name: format!("session-chat-denial-other-{}", Uuid::new_v4()),
                default_branch: GitRef::parse("refs/heads/main").expect("denial other ref"),
                is_public: false,
                agent_runs_enabled: false,
            })
            .await
            .expect("create denial-probe comparison repository");
        let other_id = other.id.as_uuid();
        let other_checkout = root.join("session-chat-denial-other-checkout");
        initialize_session_checkout(
            &other_checkout,
            source_root,
            other_id,
            git_token,
            running,
            &format!("user:{}", identity.user_id),
        )
        .await;
        let prepared = prepare_denial_probe_source(source_root, root);
        (Some(other_id), Some(prepared))
    } else {
        (None, None)
    };
    let build_context = crate::golden_modules::cooking_builds::CookingBuildContext {
        pool,
        running,
        root,
        source_root,
        project_id: project,
        repositories,
        identity: crate::golden_modules::cooking_builds::CookingIdentity {
            actor: identity,
            git_token,
            rpc_token,
        },
        timeout,
    };
    let built = crate::golden_modules::cooking_builds::build_one(
        &build_context,
        "session-chat",
        denial_source_root
            .clone()
            .unwrap_or_else(|| source_root.to_path_buf()),
        "Reference session chat agent",
        false,
    )
    .await
    .expect("build and publish session-chat release through production workers");
    assert!(!built.release_id.is_nil());
    assert!(!built.release_agent_id.is_nil());
    grant_session_capability(pool, project, identity).await;
    let denial_source_repository_id = denial_probe.then_some(built.repository_id.as_uuid());
    if browser_e2e {
        eprintln!("HEPH_SESSION_CHAT_BROWSER stage=branch-selected mode=session_chat_new");
        let existing_repository_ids: HashSet<Uuid> =
            sqlx::query_scalar("SELECT id FROM repositories WHERE project_id = $1")
                .bind(project.as_uuid())
                .fetch_all(pool)
                .await
                .expect("existing session browser repositories")
                .into_iter()
                .collect();
        let model_import = seed_model_import(pool, organization, project, identity, "model").await;
        run_session_chat_browser(
            database_url,
            running,
            project,
            built.release_agent_id,
            model_import,
            SessionChatBrowserMode::New,
        )
        .await;
        if restart_e2e_enabled() {
            broker.wait_for_observed(2).await;
            let requests = broker.observed_snapshot();
            assert_browser_session(
                pool,
                root,
                project,
                identity.user_id.as_uuid(),
                built.release_agent_id,
                &existing_repository_ids,
                requests,
            )
            .await;
            let state = load_browser_restart_state(
                pool,
                root,
                project,
                organization,
                source_root,
                identity,
                git_token,
                rpc_token,
                identity.user_id.as_uuid(),
                built.release_id,
                built.release_agent_id,
                &existing_repository_ids,
                broker,
            )
            .await;
            eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-state-captured turns=2");
            return Some(state);
        }
        let requests = broker.assert_observed().await;
        assert_browser_session(
            pool,
            root,
            project,
            identity.user_id.as_uuid(),
            built.release_agent_id,
            &existing_repository_ids,
            requests,
        )
        .await;
        return None;
    }

    let state = super::nonbrowser_setup::prepare_nonbrowser(
        pool,
        running,
        root,
        source_root,
        project,
        organization,
        repositories,
        identity,
        git_token,
        rpc_token,
        timeout,
        built,
        denial_probe,
        denial_source_repository_id,
        denial_other_repository_id,
    )
    .await;
    super::nonbrowser_assertions::verify_nonbrowser(
        pool,
        root,
        identity.user_id.as_uuid(),
        broker,
        state,
    )
    .await;
    None
}
