//! Production build and artifact retrieval proof for the generated cooking blog.
//!
//! The caller supplies the commit produced by the authorized result
//! publication.  This helper asks the Build service to build that exact Git
//! object, publishes the resulting immutable release, and reads the generated
//! HTML through the authenticated Artifact service.

use super::cooking_builds::{BuildError, CookingBuildContext, PreparedCookingBlog};
use identity_domain::UserId;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::{artifact::v1::ArtifactServiceClient, build::v1::BuildServiceClient},
    messages::hephaestus::{
        artifact::v1::GetArtifactPreviewRequest,
        build::v1::{BuildState, GetBuildRequest, RequestBuildRequest},
        common::v1::OpaqueId,
        release::v1::{
            ListRepositoryReleasesResponse, PublishReleaseRequest, SetDraftVersionRequest,
        },
    },
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::fmt::Write as _;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

/// Immutable identifiers and hashes for the generated blog release artifact.
#[derive(Debug, Clone)]
pub struct PublishedCookingBlogArtifact {
    /// Build created for the approved or conflict-resolved source commit.
    pub build_id: Uuid,
    /// Published immutable release containing the generated site.
    pub release_id: Uuid,
    /// Artifact returned by the authorized Artifact service.
    pub artifact_id: Uuid,
    /// Exact Git object consumed by the build.
    pub source_commit: String,
    /// Hash returned by the Artifact service and checked against the database.
    pub sha256: String,
}

/// Builds and publishes the blog at `source_commit`, then verifies the
/// generated recipe page through the production artifact read boundary.
#[allow(clippy::too_many_lines)] // This is a deliberately linear protocol proof.
pub async fn build_publish_and_verify(
    context: &CookingBuildContext<'_>,
    blog: &PreparedCookingBlog,
    source_commit: &str,
    expected_recipe_title: &str,
    outsider_id: UserId,
) -> Result<PublishedCookingBlogArtifact, BuildError> {
    if !matches!(source_commit.len(), 40 | 64)
        || !source_commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_state(
            "approved blog source commit is not canonical hex",
        ));
    }
    let (build_definition_hash, configuration_hash) = wait_for_blog_configuration(
        context.pool,
        blog.repository_id,
        &blog.source_commit,
        context.timeout,
    )
    .await?;
    let build_client = rpc_build_client(context, "/hephaestus.build.v1.BuildService/RequestBuild")?;
    let requested = build_client
        .request_build(RequestBuildRequest {
            context: request_context("cooking-blog-artifact-build").into(),
            repository_id: opaque(blog.repository_id.as_uuid()).into(),
            source_commit: source_commit.to_owned(),
            build_definition_hash,
            configuration_hash,
            ..Default::default()
        })
        .await?
        .into_owned();
    let build_id = response_id(requested.build_id.as_option().cloned(), "blog RequestBuild")?;
    let build = wait_for_build(context, build_id).await?;
    assert_eq!(
        build.source_commit, source_commit,
        "blog build must consume the authorized approved source commit"
    );
    let release_id = response_id(build.release_id.as_option().cloned(), "blog build release")?;

    let version = format!("v1.0.0-blog-{}", &source_commit[..12]);
    let release_client = rpc_release_client(
        context,
        "/hephaestus.release.v1.ReleaseService/SetDraftVersion",
    )?;
    let draft = release_client
        .set_draft_version(SetDraftVersionRequest {
            context: request_context("publish-cooking-blog-version").into(),
            release_id: opaque(release_id).into(),
            version,
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("blog SetDraftVersion returned no release"))?;
    if draft.state.to_i32() != 1 {
        return Err(invalid_state(
            "blog SetDraftVersion did not retain draft state",
        ));
    }
    let release_client = rpc_release_client(
        context,
        "/hephaestus.release.v1.ReleaseService/PublishRelease",
    )?;
    let published = release_client
        .publish_release(PublishReleaseRequest {
            context: request_context("publish-cooking-blog-release").into(),
            release_id: opaque(release_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("blog PublishRelease returned no release"))?;
    if published.state.to_i32() != 2 {
        return Err(invalid_state("blog release did not become published"));
    }

    let (artifact_id, expected_hash): (Uuid, String) = sqlx::query_as(
        "SELECT id, encode(content_hash, 'hex')
           FROM release_artifacts
          WHERE release_id = $1
            AND path = 'public/recipes/recipe-42/index.html'",
    )
    .bind(release_id)
    .fetch_one(context.pool)
    .await?;
    let artifact_client = rpc_artifact_client(context)?;
    let preview = artifact_client
        .get_artifact_preview(GetArtifactPreviewRequest {
            artifact_id: opaque(artifact_id).into(),
            max_bytes: 1024 * 1024,
            ..Default::default()
        })
        .await?
        .into_owned();
    let artifact = preview
        .artifact
        .as_option()
        .ok_or_else(|| invalid_state("blog artifact preview returned no metadata"))?;
    let artifact_id_string = artifact_id.to_string();
    assert_eq!(
        artifact.id.as_option().map(|id| id.value.as_str()),
        Some(artifact_id_string.as_str())
    );
    assert_eq!(artifact.path, "public/recipes/recipe-42/index.html");
    assert_eq!(artifact.sha256, expected_hash);
    let body_hash = Sha256::digest(preview.utf8_contents.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hash, byte| {
            write!(&mut hash, "{byte:02x}").expect("writing a hash to String cannot fail");
            hash
        });
    assert_eq!(artifact.sha256, body_hash);
    let provenance = artifact
        .provenance
        .as_option()
        .ok_or_else(|| invalid_state("blog artifact preview returned no provenance"))?;
    assert_eq!(
        opaque_value(provenance.build_id.as_option()),
        build_id.to_string()
    );
    assert_eq!(
        opaque_value(provenance.release_id.as_option()),
        release_id.to_string()
    );
    assert_eq!(provenance.source_commit, source_commit);
    assert!(!preview.truncated);
    assert!(preview.utf8_contents.contains("<html"));
    assert!(preview.utf8_contents.contains(expected_recipe_title));
    assert!(!preview.utf8_contents.contains("<script"));
    super::cooking::assert_no_credentials(&preview.utf8_contents);

    assert_outsider_cannot_read_release(
        context,
        blog.repository_id.as_uuid(),
        release_id,
        artifact_id,
        expected_recipe_title,
        outsider_id,
    )
    .await?;

    Ok(PublishedCookingBlogArtifact {
        build_id,
        release_id,
        artifact_id,
        source_commit: source_commit.to_owned(),
        sha256: expected_hash,
    })
}

async fn assert_outsider_cannot_read_release(
    context: &CookingBuildContext<'_>,
    repository_id: Uuid,
    release_id: Uuid,
    artifact_id: Uuid,
    expected_recipe_title: &str,
    outsider_id: UserId,
) -> Result<(), BuildError> {
    let client = reqwest::Client::new();
    let base = format!("http://{}", context.running.http_addr());
    let release = client
        .post(format!(
            "{base}/hephaestus.release.v1.ReleaseService/GetRelease"
        ))
        .bearer_auth(outsider_assertion(
            "/hephaestus.release.v1.ReleaseService/GetRelease",
            outsider_id,
        ))
        .json(&serde_json::json!({
            "releaseId": {"value": release_id.to_string()}
        }))
        .send()
        .await?;
    assert_not_found_without_artifact(release, expected_recipe_title).await?;

    let releases = client
        .post(format!(
            "{base}/hephaestus.release.v1.ReleaseService/ListRepositoryReleases"
        ))
        .bearer_auth(outsider_assertion(
            "/hephaestus.release.v1.ReleaseService/ListRepositoryReleases",
            outsider_id,
        ))
        .json(&serde_json::json!({
            "repositoryId": {"value": repository_id.to_string()},
            "page": {"pageSize": 50}
        }))
        .send()
        .await?;
    assert_eq!(releases.status(), reqwest::StatusCode::OK);
    let releases_bytes = releases.bytes().await?;
    let releases_message: ListRepositoryReleasesResponse = serde_json::from_slice(&releases_bytes)?;
    assert!(releases_message.releases.is_empty());
    let releases_body: serde_json::Value = serde_json::from_slice(&releases_bytes)?;
    assert!(!releases_body.to_string().contains(&release_id.to_string()));
    assert!(!releases_body.to_string().contains(&artifact_id.to_string()));

    let preview = client
        .post(format!(
            "{base}/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview"
        ))
        .bearer_auth(outsider_assertion(
            "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview",
            outsider_id,
        ))
        .json(&serde_json::json!({
            "artifactId": {"value": artifact_id.to_string()},
            "maxBytes": 1024
        }))
        .send()
        .await?;
    assert_not_found_without_artifact(preview, expected_recipe_title).await
}

async fn assert_not_found_without_artifact(
    response: reqwest::Response,
    expected_recipe_title: &str,
) -> Result<(), BuildError> {
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next()),
        Some("application/json")
    );
    let error: serde_json::Value = response.json().await?;
    assert_eq!(error["code"], "not_found");
    assert!(error.get("artifact").is_none());
    assert!(error.get("utf8Contents").is_none());
    assert!(!error.to_string().contains(expected_recipe_title));
    Ok(())
}

async fn wait_for_blog_configuration(
    pool: &PgPool,
    repository_id: forge_domain::RepositoryId,
    source_commit: &str,
    timeout: Duration,
) -> Result<(String, String), BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(hashes) = sqlx::query_as::<_, (String, String)>(
            "SELECT encode(build_definition_hash, 'hex'), encode(configuration_hash, 'hex')
               FROM build_requests
              WHERE repository_id = $1 AND source_commit = $2
              ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(repository_id.as_uuid())
        .bind(source_commit)
        .fetch_optional(pool)
        .await?
        {
            return Ok(hashes);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state(
                "timed out waiting for cooking blog build configuration",
            ));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_for_build(
    context: &CookingBuildContext<'_>,
    build_id: Uuid,
) -> Result<rpc_proto::messages::hephaestus::build::v1::Build, BuildError> {
    let deadline = tokio::time::Instant::now() + context.timeout;
    loop {
        let client = rpc_build_client(context, "/hephaestus.build.v1.BuildService/GetBuild")?;
        let build = client
            .get_build(GetBuildRequest {
                build_id: opaque(build_id).into(),
                ..Default::default()
            })
            .await?
            .into_owned()
            .build
            .into_option()
            .ok_or_else(|| invalid_state("GetBuild returned no cooking blog build"))?;
        match build.state {
            state if state.to_i32() == BuildState::BUILD_STATE_SUCCEEDED as i32 => {
                return Ok(build);
            }
            state if state.to_i32() == BuildState::BUILD_STATE_FAILED as i32 => {
                return Err(invalid_state("cooking blog artifact build failed"));
            }
            _ if tokio::time::Instant::now() >= deadline => {
                return Err(invalid_state(
                    "timed out waiting for cooking blog artifact build",
                ));
            }
            _ => sleep(Duration::from_millis(500)).await,
        }
    }
}

fn rpc_build_client(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<BuildServiceClient<connectrpc::client::HttpClient>, BuildError> {
    client_with_audience(context, audience)
        .map(|(transport, config)| BuildServiceClient::new(transport, config))
}

fn rpc_artifact_client(
    context: &CookingBuildContext<'_>,
) -> Result<ArtifactServiceClient<connectrpc::client::HttpClient>, BuildError> {
    client_with_audience(
        context,
        "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview",
    )
    .map(|(transport, config)| ArtifactServiceClient::new(transport, config))
}

fn rpc_release_client(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<
    rpc_proto::connect::hephaestus::release::v1::ReleaseServiceClient<
        connectrpc::client::HttpClient,
    >,
    BuildError,
> {
    client_with_audience(context, audience).map(|(transport, config)| {
        rpc_proto::connect::hephaestus::release::v1::ReleaseServiceClient::new(transport, config)
    })
}

fn client_with_audience(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<
    (
        connectrpc::client::HttpClient,
        connectrpc::client::ClientConfig,
    ),
    BuildError,
> {
    let uri = format!("http://{}", context.running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!(
                "Bearer {}",
                (context.identity.rpc_token)(audience)
            ))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok((connectrpc::client::HttpClient::plaintext(), config))
}

fn request_context(operation: &str) -> rpc_proto::messages::hephaestus::common::v1::RequestContext {
    rpc_proto::messages::hephaestus::common::v1::RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("cooking-blog-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn response_id(value: Option<OpaqueId>, operation: &str) -> Result<Uuid, BuildError> {
    value
        .ok_or_else(|| invalid_state(&format!("{operation} returned no ID")))?
        .value
        .parse()
        .map_err(Into::into)
}

fn opaque_value(value: Option<&OpaqueId>) -> String {
    value.map_or_else(String::new, |id| id.value.clone())
}

fn invalid_state(message: &str) -> BuildError {
    message.to_owned().into()
}

fn outsider_assertion(audience: &str, outsider_id: UserId) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": outsider_id.to_string(),
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 30,
            "jti": Uuid::new_v4().to_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign outsider artifact assertion")
}
