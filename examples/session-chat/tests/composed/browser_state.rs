use super::*;
use super::{SessionBrokerFixture, SessionChatBrowserMode};
use forge_domain::ProjectId;
use hephaestus_app::RunningHephaestus;
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use rpc_proto::messages::hephaestus::common::v1::RequestContext;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};
use time::OffsetDateTime;
use uuid::Uuid;
/// Persisted identifiers and immutable first-phase evidence used by recovery.
pub(crate) struct BrowserRestartState<'a> {
    pub(super) broker: SessionBrokerFixture,
    pub(super) project: ProjectId,
    pub(super) organization: OrganizationId,
    pub(super) source_root: &'a Path,
    pub(super) identity: &'a AuthenticatedIdentity,
    pub(super) git_token: &'a str,
    pub(super) rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    pub(super) actor_id: Uuid,
    pub(super) release_id: Uuid,
    pub(super) release_agent_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) instance_id: Uuid,
    pub(super) revision_id: Uuid,
    pub(super) attachment_id: Uuid,
    pub(super) installation_id: Uuid,
    pub(super) generation_id: Uuid,
    pub(super) session_id: Uuid,
    pub(super) initial_head: String,
    pub(super) initial_receive_count: i64,
    pub(super) initial_record_blobs: HashMap<String, Vec<u8>>,
    pub(super) previous_runtime_session_ids: Vec<Uuid>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One exact persisted-session lookup keeps restart identity immutable.
pub(crate) async fn load_browser_restart_state<'a>(
    pool: &PgPool,
    root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    source_root: &'a Path,
    identity: &'a AuthenticatedIdentity,
    git_token: &'a str,
    rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    actor_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    existing_repository_ids: &HashSet<Uuid>,
    broker: SessionBrokerFixture,
) -> BrowserRestartState<'a> {
    let candidates: Vec<BrowserSessionObjects> = sqlx::query_as(
        "SELECT repository.id AS repository_id,
                instance.id AS instance_id,
                revision.id AS revision_id,
                attachment.id AS attachment_id
           FROM repositories repository
           JOIN agent_attachments attachment
             ON attachment.repository_id = repository.id
            AND attachment.project_id = repository.project_id
           JOIN agent_instances instance
             ON instance.id = attachment.instance_id
            AND instance.project_id = repository.project_id
           JOIN agent_instance_revisions revision
             ON revision.id = instance.active_revision_id
            AND revision.instance_id = instance.id
            AND revision.release_agent_id = $2
          WHERE repository.project_id = $1
            AND attachment.ref_selector = 'refs/heads/main'
            AND attachment.removed_at IS NULL",
    )
    .bind(project.as_uuid())
    .bind(release_agent_id)
    .fetch_all(pool)
    .await
    .expect("browser restart repository and attachment");
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| !existing_repository_ids.contains(&candidate.repository_id))
        .collect();
    assert_eq!(
        candidates.len(),
        1,
        "browser restart must retain exactly one session repository"
    );
    let BrowserSessionObjects {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
    } = candidates
        .into_iter()
        .next()
        .expect("browser restart session repository");
    let (installation_id, generation_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT installation.id, installation.current_generation_id
           FROM ui_installations installation
           JOIN ui_installation_generations generation
             ON generation.id = installation.current_generation_id
            AND generation.installation_id = installation.id
          WHERE installation.project_id = $1
            AND installation.repository_id = $2
            AND installation.scope = 'repository'
            AND installation.ui_key = 'session-chat'
            AND installation.lifecycle = 'enabled'
            AND generation.release_id = $3
            AND generation.ui_key = 'session-chat'
            AND generation.ui_scope = 'repository'",
    )
    .bind(project.as_uuid())
    .bind(repository_id)
    .bind(release_id)
    .fetch_one(pool)
    .await
    .expect("browser restart installed UI generation");
    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    let manifest_path = ".heph/session/v1/manifest.json";
    let manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            repository_id,
            &["show", &format!("{head}:{manifest_path}")],
        )
        .await,
    )
    .expect("browser restart session manifest JSON");
    let session_id = Uuid::parse_str(
        manifest["data"]["session_id"]
            .as_str()
            .expect("browser restart session ID"),
    )
    .expect("browser restart session UUID");
    let initial_receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("browser restart initial receive count");
    let record_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", &head],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut initial_record_blobs = HashMap::with_capacity(record_paths.len());
    for path in record_paths {
        let content =
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await;
        initial_record_blobs.insert(path, content);
    }
    let previous_runtime_session_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT session.id
           FROM runtime_authority_sessions AS session
           JOIN run_requests AS request ON request.run_id = session.run_id
          WHERE session.instance_id = $1
            AND session.attachment_id = $2
            AND request.request_kind = 'instance_normal'
          ORDER BY session.created_at, session.id",
    )
    .bind(instance_id)
    .bind(attachment_id)
    .fetch_all(pool)
    .await
    .expect("browser restart previous runtime authority sessions");
    let previous_publication_session_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT session.id
           FROM runtime_authority_sessions AS session
           JOIN git_receives AS receive
             ON receive.runtime_session_id = session.id
          WHERE session.instance_id = $1
            AND session.attachment_id = $2
            AND receive.runtime_attachment_id = $2
            AND receive.status = 'accepted'
          ORDER BY session.id",
    )
    .bind(instance_id)
    .bind(attachment_id)
    .fetch_all(pool)
    .await
    .expect("browser restart previous publication runtime sessions");
    assert_eq!(
        previous_publication_session_ids.len(),
        2,
        "first browser phase must publish two runtime-authenticated turns"
    );
    BrowserRestartState {
        broker,
        project,
        organization,
        source_root,
        identity,
        git_token,
        rpc_token,
        actor_id,
        release_id,
        release_agent_id,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        installation_id,
        generation_id,
        session_id,
        initial_head: head,
        initial_receive_count,
        initial_record_blobs,
        previous_runtime_session_ids,
    }
}
