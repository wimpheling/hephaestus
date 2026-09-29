use super::{
    BuildError, CookingBuildContext, PreparedCookingBlog, PublishedCookingBlogArtifact,
    assert_outsider_cannot_read_release, invalid_state, opaque, opaque_value, request_context,
    response_id, rpc_artifact_client, rpc_build_client, rpc_release_client,
    wait_for_blog_configuration, wait_for_build,
};
use identity_domain::{BrowserSessionSid, UserId};
use rpc_proto::messages::hephaestus::{
    artifact::v1::GetArtifactPreviewRequest,
    build::v1::RequestBuildRequest,
    release::v1::{PublishReleaseRequest, SetDraftVersionRequest},
};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use uuid::Uuid;

/// Builds and publishes the blog at `source_commit`, then verifies the
/// generated recipe page through the production artifact read boundary.
#[allow(clippy::too_many_lines)] // This is a deliberately linear protocol proof.
pub async fn build_publish_and_verify(
    context: &CookingBuildContext<'_>,
    blog: &PreparedCookingBlog,
    source_commit: &str,
    expected_recipe_title: &str,
    outsider_id: UserId,
    outsider_browser_session: BrowserSessionSid,
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
    super::super::cooking::assert_no_credentials(&preview.utf8_contents);

    assert_outsider_cannot_read_release(
        context,
        blog.repository_id.as_uuid(),
        release_id,
        artifact_id,
        expected_recipe_title,
        outsider_id,
        outsider_browser_session,
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
