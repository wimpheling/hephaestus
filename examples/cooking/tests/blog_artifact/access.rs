use super::{BuildError, CookingBuildContext, outsider_assertion};
use identity_domain::{BrowserSessionSid, UserId};
use rpc_proto::messages::hephaestus::release::v1::ListRepositoryReleasesResponse;
use uuid::Uuid;

pub(crate) async fn assert_outsider_cannot_read_release(
    context: &CookingBuildContext<'_>,
    repository_id: Uuid,
    release_id: Uuid,
    artifact_id: Uuid,
    expected_recipe_title: &str,
    outsider_id: UserId,
    outsider_browser_session: BrowserSessionSid,
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
            outsider_browser_session,
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
            outsider_browser_session,
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
            outsider_browser_session,
        ))
        .json(&serde_json::json!({
            "artifactId": {"value": artifact_id.to_string()},
            "maxBytes": 1024
        }))
        .send()
        .await?;
    assert_not_found_without_artifact(preview, expected_recipe_title).await
}

pub(crate) async fn assert_not_found_without_artifact(
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
