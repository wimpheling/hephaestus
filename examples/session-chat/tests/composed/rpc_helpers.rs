#![allow(unused_imports)]
use super::*;
use hephaestus_app::RunningHephaestus;
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::common::v1::{OpaqueId, RequestContext},
};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;
pub(crate) fn instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<
    AgentInstanceServiceClient<connectrpc::client::HttpClient>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(AgentInstanceServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

pub(crate) fn mutation_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("session-chat-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

pub(crate) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(crate) fn response_id(
    value: Option<OpaqueId>,
    operation: &str,
) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
    value
        .ok_or_else(|| format!("{operation} returned no ID"))?
        .value
        .parse()
        .map_err(Into::into)
}

pub(crate) async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run session Git command");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) async fn authenticated_git(directory: &Path, token: &str, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run authenticated session Git command");
    assert!(
        output.status.success(),
        "authenticated Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run session Git output command");
    assert!(
        output.status.success(),
        "Git output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("session Git output UTF-8")
        .trim()
        .to_owned()
}

pub(crate) async fn git_output_bare(root: &Path, repository: Uuid, arguments: &[&str]) -> String {
    let bare = root.join("repositories").join(format!("{repository}.git"));
    let output = Command::new("git")
        .arg(format!("--git-dir={}", bare.display()))
        .args(arguments)
        .output()
        .await
        .expect("run session bare Git output command");
    assert!(
        output.status.success(),
        "bare Git output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("session bare Git output UTF-8")
        .trim()
        .to_owned()
}

pub(crate) async fn git_output_bare_bytes(
    root: &Path,
    repository: Uuid,
    arguments: &[&str],
) -> Vec<u8> {
    let bare = root.join("repositories").join(format!("{repository}.git"));
    let output = Command::new("git")
        .arg(format!("--git-dir={}", bare.display()))
        .args(arguments)
        .output()
        .await
        .expect("run session bare Git byte output command");
    assert!(
        output.status.success(),
        "bare Git byte output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
