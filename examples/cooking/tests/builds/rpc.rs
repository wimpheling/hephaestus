// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
pub(crate) fn rpc_build_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
) -> Result<BuildServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!(
                "Bearer {}",
                token_factory("/hephaestus.build.v1.BuildService/GetBuild")
            ))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    let transport = connectrpc::client::HttpClient::plaintext();
    Ok(BuildServiceClient::new(transport, config))
}

pub(crate) fn rpc_gateway_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(GatewayServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

pub(crate) fn rpc_instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<AgentInstanceServiceClient<connectrpc::client::HttpClient>, BuildError> {
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

pub(crate) fn rpc_release_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<ReleaseServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(ReleaseServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

pub(crate) async fn wait_for_build_row(
    pool: &PgPool,
    repository_id: RepositoryId,
    source_commit: &str,
    timeout: Duration,
) -> Result<Uuid, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2
               AND source_ref = 'refs/heads/main'
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(repository_id.as_uuid())
        .bind(source_commit)
        .fetch_optional(pool)
        .await?
        {
            return Ok(id);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state("Git push did not create a build request"));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

pub(crate) async fn wait_for_successful_build(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    build_request_id: Uuid,
    timeout: Duration,
) -> Result<rpc_proto::messages::hephaestus::build::v1::Build, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let client = rpc_build_client(running, token_factory)?;
        let build = client
            .get_build(GetBuildRequest {
                build_id: opaque(build_request_id).into(),
                ..Default::default()
            })
            .await?
            .into_owned()
            .build
            .into_option()
            .ok_or_else(|| invalid_state("GetBuild returned no build"))?;
        match build.state.to_i32() {
            value if value == BuildState::BUILD_STATE_SUCCEEDED as i32 => return Ok(build),
            value
                if value == BuildState::BUILD_STATE_FAILED as i32
                    || value == BuildState::BUILD_STATE_CANCELLED as i32 =>
            {
                let logs = build
                    .logs
                    .iter()
                    .map(|line| redact_build_log(line))
                    .collect::<Vec<_>>()
                    .join("\\n");
                return Err(invalid_state(&format!(
                    "cooking isolated build failed: code={} exit={:?} logs={logs}",
                    build.failure_code, build.exit_code
                )));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state(
                "timed out waiting for cooking isolated build",
            ));
        }
        sleep(Duration::from_millis(500)).await;
    }
}

pub(crate) async fn wait_for_draft_release(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    release_id: Uuid,
    timeout: Duration,
) -> Result<rpc_proto::messages::hephaestus::release::v1::Release, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let client = rpc_release_client(
            running,
            token_factory,
            "/hephaestus.release.v1.ReleaseService/GetRelease",
        )?;
        let release = client
            .get_release(GetReleaseRequest {
                release_id: opaque(release_id).into(),
                ..Default::default()
            })
            .await?
            .into_owned()
            .release
            .into_option()
            .ok_or_else(|| invalid_state("GetRelease returned no release"))?;
        if release.state.to_i32() == 1 {
            return Ok(release);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state("timed out waiting for draft cooking release"));
        }
        sleep(Duration::from_millis(500)).await;
    }
}

pub(crate) fn mutation_context(operation: &str) -> RequestContext {
    mutation_context_with_key(&format!("cooking-build-{operation}-{}", Uuid::new_v4()))
}

pub(crate) fn mutation_context_with_key(key: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

pub(crate) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(crate) fn response_id(value: Option<OpaqueId>, operation: &str) -> Result<Uuid, BuildError> {
    value
        .ok_or_else(|| invalid_state(&format!("{operation} returned no ID")))?
        .value
        .parse::<Uuid>()
        .map_err(Into::into)
}
