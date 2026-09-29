use super::model::ObservedModelRequest;
use super::*;
use super::{BrowserSessionObjects, MODEL_RESPONSE_TEXT};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use uuid::Uuid;

pub(crate) struct BrowserRecordEvidence {
    pub(crate) repository_id: Uuid,
    pub(crate) instance_id: Uuid,
    pub(crate) revision_id: Uuid,
    pub(crate) attachment_id: Uuid,
    pub(crate) session_id: Uuid,
    pub(crate) human_records: Vec<(Uuid, JsonValue)>,
    pub(crate) human_record_paths: HashMap<Uuid, String>,
    pub(crate) agent_records: Vec<(Uuid, JsonValue)>,
    pub(crate) agent_record_paths: HashMap<Uuid, String>,
}

#[allow(clippy::too_many_lines)]
// Keep the complete record snapshot query and decoding together for restart comparison.
pub(crate) async fn load_browser_records(
    pool: &PgPool,
    root: &Path,
    project: forge_domain::ProjectId,
    release_agent_id: Uuid,
    existing_repository_ids: &HashSet<Uuid>,
    requests: &[ObservedModelRequest],
) -> BrowserRecordEvidence {
    assert_eq!(
        requests.len(),
        2,
        "browser session must make two model turns"
    );
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
    .expect("browser session repository and attachment");
    let new_candidates: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| !existing_repository_ids.contains(&candidate.repository_id))
        .collect();
    assert_eq!(
        new_candidates.len(),
        1,
        "browser session must create one project repository attached to the selected release"
    );
    let BrowserSessionObjects {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
    } = new_candidates
        .into_iter()
        .next()
        .expect("new browser session");
    assert!(!instance_id.is_nil());
    assert!(!revision_id.is_nil());
    assert!(!attachment_id.is_nil());

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"])
        .await
        .trim()
        .to_owned();
    let paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", head.trim()],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let manifest_path = ".heph/session/v1/manifest.json";
    assert!(paths.iter().any(|path| path == manifest_path));
    let manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            repository_id,
            &["show", &format!("{head}:{manifest_path}")],
        )
        .await,
    )
    .expect("browser session manifest JSON");
    let session_id = Uuid::parse_str(
        manifest["data"]["session_id"]
            .as_str()
            .expect("browser session manifest session ID"),
    )
    .expect("browser session manifest session UUID");

    let human_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .collect::<Vec<_>>();
    let agent_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        2,
        "browser session must publish two human records"
    );
    assert_eq!(
        agent_paths.len(),
        2,
        "browser session must publish two assistant records"
    );
    let mut human_records = Vec::with_capacity(human_paths.len());
    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("browser human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("browser human record ID"),
        )
        .expect("browser human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, (*path).clone());
        human_records.push((record_id, record));
    }
    let mut agent_records = Vec::with_capacity(agent_paths.len());
    let mut agent_record_paths = HashMap::with_capacity(agent_paths.len());
    for path in agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("browser assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("browser assistant record ID"),
        )
        .expect("browser assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, (*path).clone());
        agent_records.push((record_id, record));
    }

    BrowserRecordEvidence {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        session_id,
        human_records,
        human_record_paths,
        agent_records,
        agent_record_paths,
    }
}
