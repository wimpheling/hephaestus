use run_orchestrator::{
    MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeCatalogError,
};
use serde_json::Value;
use sha2::Digest;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub(super) struct RuntimeContextRow {
    pub(super) parameters: Value,
    pub(super) repository_id: Option<Uuid>,
    pub(super) git_ref: Option<String>,
    pub(super) commit_sha: Option<String>,
    pub(super) release_state: String,
    pub(super) update_id: Option<Uuid>,
    pub(super) previous_revision_id: Option<Uuid>,
    pub(super) previous_release_id: Option<Uuid>,
    pub(super) previous_parameters: Option<Value>,
}

#[derive(Debug, FromRow)]
pub(super) struct RuntimeArtifactRow {
    pub(super) path: String,
    pub(super) kind: String,
    pub(super) mode: i32,
    pub(super) content_hash: Vec<u8>,
    pub(super) size_bytes: i64,
    pub(super) storage_key: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct MailboxRuntimeRow {
    pub(super) mailbox_id: Uuid,
    pub(super) event_id: Uuid,
    pub(super) body_id: Uuid,
    pub(super) method: String,
    pub(super) route: String,
    pub(super) selected_headers: Value,
    pub(super) content_type: Option<String>,
    pub(super) trace_context: Option<String>,
    pub(super) received_at: time::OffsetDateTime,
    pub(super) encoded_body: Option<Vec<u8>>,
    pub(super) integrity_hash: Vec<u8>,
}

impl TryFrom<MailboxRuntimeRow> for MailboxRuntimeEvent {
    type Error = RunRuntimeCatalogError;

    fn try_from(row: MailboxRuntimeRow) -> Result<Self, Self::Error> {
        let body = row.encoded_body.ok_or(RunRuntimeCatalogError::InvalidData(
            "mailbox body was purged",
        ))?;
        if body.len() > 1_048_576 {
            return Err(RunRuntimeCatalogError::InvalidData("mailbox body size"));
        }
        let integrity_hash = row
            .integrity_hash
            .try_into()
            .map_err(|_| RunRuntimeCatalogError::InvalidData("mailbox body hash"))?;
        if sha2::Sha256::digest(&body).as_slice() != integrity_hash {
            return Err(RunRuntimeCatalogError::InvalidData(
                "mailbox body integrity",
            ));
        }
        Ok(Self {
            mailbox_id: row.mailbox_id,
            event_id: row.event_id,
            body_id: row.body_id,
            method: row.method,
            route: row.route,
            selected_headers: row.selected_headers,
            content_type: row.content_type,
            trace_context: row.trace_context,
            received_at: row.received_at,
            body,
            integrity_hash,
        })
    }
}

impl TryFrom<RuntimeArtifactRow> for RunRuntimeArtifact {
    type Error = RunRuntimeCatalogError;

    fn try_from(row: RuntimeArtifactRow) -> Result<Self, Self::Error> {
        let kind = match row.kind.as_str() {
            "executable" => RunRuntimeArtifactKind::Executable,
            "file" => RunRuntimeArtifactKind::File,
            "manifest" => RunRuntimeArtifactKind::Manifest,
            _ => return Err(RunRuntimeCatalogError::InvalidData("artifact kind")),
        };
        let content_hash = row
            .content_hash
            .try_into()
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact content hash"))?;
        let mode = u32::try_from(row.mode)
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact mode"))?;
        let size_bytes = u64::try_from(row.size_bytes)
            .map_err(|_| RunRuntimeCatalogError::InvalidData("artifact size"))?;
        Ok(Self {
            path: row.path,
            kind,
            mode,
            content_hash,
            size_bytes,
            storage_key: row.storage_key,
        })
    }
}
