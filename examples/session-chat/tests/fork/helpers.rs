use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use buffa_types::google::protobuf::Timestamp;
use forge_domain::ProjectId;
use hephaestus_app::RunningHephaestus;
use rpc_proto::{
    connect::hephaestus::{
        pat::v1::PersonalAccessTokenServiceClient, release::v1::ReleaseServiceClient,
        repository::v1::RepositoryServiceClient,
    },
    messages::hephaestus::pat::v1::{
        CreatePersonalAccessTokenRequest, GitOperation, PersonalAccessTokenScope,
    },
    messages::hephaestus::repository::v1::CreateRepositoryRequest,
};
use std::{collections::BTreeSet, path::Path, process::Stdio, time::Duration};
use time::OffsetDateTime;
use tokio::process::Command;
use uuid::Uuid;

pub(super) async fn create_target_repository(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    project: ProjectId,
) -> Uuid {
    let audience = "/hephaestus.repository.v1.RepositoryService/CreateRepository";
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("repository RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("repository RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    let client = RepositoryServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    let response = client
        .create_repository(CreateRepositoryRequest {
            context: super::super::mutation_context("fork-target-repository").into(),
            project_id: super::super::opaque(project.as_uuid()).into(),
            name: format!("session-chat-fork-{}", Uuid::new_v4()),
            default_branch: String::from("main"),
            is_public: false,
            agent_runs_enabled: true,
            ..Default::default()
        })
        .await
        .expect("create fork target repository through RPC")
        .into_owned();
    super::super::response_id(
        response.repository_id.into_option(),
        "fork target repository",
    )
    .expect("fork target repository ID")
}

pub(super) fn release_client(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> ReleaseServiceClient<connectrpc::client::HttpClient> {
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("release RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("release RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    ReleaseServiceClient::new(connectrpc::client::HttpClient::plaintext(), config)
}

pub(super) async fn issue_target_pat(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    repository_id: Uuid,
) -> String {
    let audience = "/hephaestus.pat.v1.PersonalAccessTokenService/CreatePersonalAccessToken";
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("PAT RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("PAT RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    let client =
        PersonalAccessTokenServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    let expires_at = OffsetDateTime::now_utc() + time::Duration::hours(1);
    let response = client
        .create_personal_access_token(CreatePersonalAccessTokenRequest {
            context: super::super::mutation_context("fork-target-pat").into(),
            label: format!("session-chat-fork-target-{repository_id}"),
            scope: PersonalAccessTokenScope {
                operations: vec![
                    GitOperation::Discover.into(),
                    GitOperation::Fetch.into(),
                    GitOperation::Receive.into(),
                ],
                repository_ids: vec![super::super::opaque(repository_id)],
                ..Default::default()
            }
            .into(),
            expires_at: Timestamp {
                seconds: expires_at.unix_timestamp(),
                nanos: i32::try_from(expires_at.nanosecond()).expect("PAT expiry nanos"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect("issue target-scoped Git PAT")
        .into_owned();
    let metadata = response.token.into_option().expect("target PAT metadata");
    let scope = metadata.scope.into_option().expect("target PAT scope");
    assert_eq!(
        scope.repository_ids,
        vec![super::super::opaque(repository_id)]
    );
    assert_eq!(
        scope
            .operations
            .iter()
            .map(buffa::enumeration::EnumValue::as_known)
            .collect::<Option<Vec<_>>>(),
        Some(vec![
            GitOperation::Discover,
            GitOperation::Fetch,
            GitOperation::Receive
        ])
    );
    let value = response
        .value
        .into_option()
        .expect("target PAT value")
        .value;
    String::from_utf8(value).unwrap_or_else(|_| panic!("target PAT response was not UTF-8"))
}

pub(super) async fn fork_local_checkout(
    source_root: &Path,
    source_clone: &Path,
    target_checkout: &Path,
    target_session_id: Uuid,
) {
    let script = r"
from pathlib import Path
import sys
from git_adapter import LocalGitSession

source = LocalGitSession.open(Path(sys.argv[1]))
source.fork(Path(sys.argv[2]), sys.argv[3])
";
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(source_clone)
        .arg(target_checkout)
        .arg(target_session_id.to_string())
        .env("PYTHONPATH", source_root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run LocalGitSession fork helper");
    assert!(
        output.status.success(),
        "LocalGitSession fork helper failed"
    );
}

pub(super) async fn authenticated_git_pat(directory: &Path, token: &str, arguments: &[&str]) {
    let basic = BASE64_STANDARD.encode(format!("heph-pat:{token}"));
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Basic {basic}"),
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run target PAT Git command");
    assert!(output.status.success(), "target PAT Git command failed");
}

pub(super) async fn commit_ids(root: &Path, repository_id: Uuid, head: &str) -> BTreeSet<String> {
    super::super::git_output_bare(root, repository_id, &["rev-list", head])
        .await
        .lines()
        .map(str::to_owned)
        .collect()
}

pub(super) async fn object_ids(root: &Path, repository_id: Uuid, head: &str) -> BTreeSet<String> {
    super::super::git_output_bare(root, repository_id, &["rev-list", "--objects", head])
        .await
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}
