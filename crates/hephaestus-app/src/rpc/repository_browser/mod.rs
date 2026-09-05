//! Repository browser RPC composition and shared conversions.

mod get_commit_detail;
mod get_file;
mod get_tree;
mod list_branches;
mod list_commits;
mod stream_file;

use super::{MediatorAuthenticator, RpcError};
use crate::application::repository_browser::{
    Branch, BrowserApplication, BrowserError, Commit as ApplicationCommit,
    CommitDetail as ApplicationCommitDetail, DiffFile as ApplicationDiffFile,
    DiffFileState as ApplicationDiffFileState, DiffHunk as ApplicationDiffHunk,
    DiffLine as ApplicationDiffLine, DiffLineKind as ApplicationDiffLineKind,
    TreeEntry as ApplicationTreeEntry,
};
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult, ServiceStream};
use control_plane_postgres::ControlPlanePool as PgPool;
use forge_service::GitStorage;
use rpc_proto::{
    connect::hephaestus::repository_browser::v1::{
        RepositoryBrowserService, RepositoryBrowserServiceExt,
    },
    messages::hephaestus::{
        common::v1::OpaqueId,
        repository_browser::v1::{
            Branch as ProtoBranch, Commit as ProtoCommit, CommitDetail, DiffFile, DiffFileState,
            DiffHunk, DiffLine, DiffLineKind, GetCommitDetailRequest, GetCommitDetailResponse,
            GetFileRequest, GetFileResponse, GetTreeRequest, GetTreeResponse, ListBranchesRequest,
            ListBranchesResponse, ListCommitsRequest, ListCommitsResponse, StreamFileRequest,
            StreamFileResponse, TreeEntry, TreeEntryType,
        },
    },
};
use std::sync::Arc;
use uuid::Uuid;

pub struct RepositoryBrowserRpc {
    application: BrowserApplication,
    authenticator: MediatorAuthenticator,
}

impl RepositoryBrowserRpc {
    const fn new(
        pool: PgPool,
        storage: Arc<GitStorage>,
        authenticator: MediatorAuthenticator,
    ) -> Self {
        Self {
            application: BrowserApplication::new(pool, storage),
            authenticator,
        }
    }
}

pub fn register(
    router: Router,
    pool: PgPool,
    storage: Arc<GitStorage>,
    authenticator: MediatorAuthenticator,
) -> Router {
    RepositoryBrowserServiceExt::register(
        Arc::new(RepositoryBrowserRpc::new(pool, storage, authenticator)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl RepositoryBrowserService for RepositoryBrowserRpc {
    async fn list_branches(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListBranchesRequest>,
    ) -> ServiceResult<ListBranchesResponse> {
        list_branches::handle(self, ctx, request).await
    }
    async fn list_commits(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListCommitsRequest>,
    ) -> ServiceResult<ListCommitsResponse> {
        list_commits::handle(self, ctx, request).await
    }
    async fn get_tree(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetTreeRequest>,
    ) -> ServiceResult<GetTreeResponse> {
        get_tree::handle(self, ctx, request).await
    }
    async fn get_file(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetFileRequest>,
    ) -> ServiceResult<GetFileResponse> {
        get_file::handle(self, ctx, request).await
    }
    async fn get_commit_detail(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetCommitDetailRequest>,
    ) -> ServiceResult<GetCommitDetailResponse> {
        get_commit_detail::handle(self, ctx, request).await
    }
    async fn stream_file(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, StreamFileRequest>,
    ) -> ServiceResult<ServiceStream<StreamFileResponse>> {
        stream_file::handle(self, ctx, request).await
    }
}

fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, RpcError> {
    value
        .ok_or(RpcError::InvalidArgument)?
        .value
        .parse()
        .map_err(|_| RpcError::InvalidArgument)
}

fn branch(value: Branch) -> ProtoBranch {
    ProtoBranch {
        name: value.name,
        r#ref: value.git_ref,
        commit: value.commit,
        committed_at: timestamp(value.committed_at).into(),
        subject: value.subject,
        ..Default::default()
    }
}

fn commit(value: ApplicationCommit) -> ProtoCommit {
    ProtoCommit {
        id: value.id,
        parents: value.parents,
        author_name: value.author_name,
        author_email: value.author_email,
        authored_at: timestamp(value.authored_at).into(),
        subject: value.subject,
        ..Default::default()
    }
}

fn commit_detail(value: ApplicationCommitDetail) -> CommitDetail {
    CommitDetail {
        commit: commit(value.commit).into(),
        committer_name: value.committer_name,
        committer_email: value.committer_email,
        committed_at: timestamp(value.committed_at).into(),
        body: value.body,
        selected_parent: value.selected_parent,
        files: value.files.into_iter().map(diff_file).collect(),
        truncated: value.truncated,
        ..Default::default()
    }
}

fn diff_file(value: ApplicationDiffFile) -> DiffFile {
    DiffFile {
        path: value.path,
        previous_path: value.previous_path,
        state: match value.state {
            ApplicationDiffFileState::Added => DiffFileState::Added,
            ApplicationDiffFileState::Deleted => DiffFileState::Deleted,
            ApplicationDiffFileState::Modified => DiffFileState::Modified,
            ApplicationDiffFileState::Renamed => DiffFileState::Renamed,
            ApplicationDiffFileState::Binary => DiffFileState::Binary,
            ApplicationDiffFileState::Truncated => DiffFileState::Truncated,
            ApplicationDiffFileState::Unavailable => DiffFileState::Unavailable,
        }
        .into(),
        additions: value.additions,
        deletions: value.deletions,
        hunks: value.hunks.into_iter().map(diff_hunk).collect(),
        binary: matches!(value.state, ApplicationDiffFileState::Binary),
        truncated: value.truncated,
        ..Default::default()
    }
}

fn diff_hunk(value: ApplicationDiffHunk) -> DiffHunk {
    DiffHunk {
        old_start: value.old_start,
        old_lines: value.old_lines,
        new_start: value.new_start,
        new_lines: value.new_lines,
        lines: value.lines.into_iter().map(diff_line).collect(),
        truncated: value.truncated,
        ..Default::default()
    }
}

fn diff_line(value: ApplicationDiffLine) -> DiffLine {
    DiffLine {
        kind: match value.kind {
            ApplicationDiffLineKind::Context => DiffLineKind::Context,
            ApplicationDiffLineKind::Added => DiffLineKind::Added,
            ApplicationDiffLineKind::Removed => DiffLineKind::Removed,
        }
        .into(),
        old_line: value.old_line,
        new_line: value.new_line,
        text: value.text,
        ..Default::default()
    }
}

fn tree_entry(value: ApplicationTreeEntry) -> Result<TreeEntry, RpcError> {
    let kind = match value.kind.as_str() {
        "blob" => TreeEntryType::Blob,
        "tree" => TreeEntryType::Tree,
        "commit" => TreeEntryType::Commit,
        _ => return Err(RpcError::Internal),
    };
    Ok(TreeEntry {
        mode: value.mode,
        r#type: kind.into(),
        object_id: value.object_id,
        size: value.size,
        path: value.path,
        ..Default::default()
    })
}

fn timestamp(seconds: i64) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds,
        ..Default::default()
    }
}

fn map_error(error: BrowserError) -> RpcError {
    match error {
        BrowserError::PermissionDenied => RpcError::PermissionDenied,
        BrowserError::NotFound => RpcError::NotFound,
        BrowserError::InvalidArgument => RpcError::InvalidArgument,
        BrowserError::ResourceExhausted => RpcError::ResourceExhausted,
        BrowserError::Persistence(source) => {
            tracing::error!(error = %source, "repository browser authorization failed");
            RpcError::Unavailable
        }
        BrowserError::Storage(source) => {
            tracing::warn!(error = %source, "repository browser storage unavailable");
            RpcError::NotFound
        }
        BrowserError::Git => RpcError::Unavailable,
    }
}

fn language(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("ex" | "exs") => "elixir",
        Some("rs") => "rust",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("json") => "json",
        Some("sql") => "sql",
        Some("sh") => "shell",
        Some("yml" | "yaml") => "yaml",
        _ => "text",
    }
}

#[cfg(test)]
mod tests {
    use super::{language, timestamp};
    #[test]
    fn language_mapping_is_bounded_to_known_labels() {
        assert_eq!(language("src/lib.rs"), "rust");
        assert_eq!(language("unknown.bin"), "text");
    }

    #[test]
    fn git_timestamp_is_constructed_without_json_round_trip() {
        let projected = timestamp(1_700_000_000);
        assert_eq!(projected.seconds, 1_700_000_000);
        assert_eq!(projected.nanos, 0);
    }
}
