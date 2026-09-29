// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Creates and pushes the separate blog repository used by the cooking
/// instance attachment.
pub(crate) async fn create_cooking_blog_repository(
    context: &CookingBuildContext<'_>,
) -> Result<PreparedCookingBlog, BuildError> {
    let source = canonical_source(context.source_root, "cooking-blog");
    create_cooking_blog_toolchain_repository(context, &source).await?;
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("cooking-blog-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            agent_runs_enabled: false,
        })
        .await?;
    let destination = context
        .root
        .join(format!("cooking-blog-{}", Uuid::new_v4()));
    copy_source_tree(&source, &destination)?;
    // The Hugo toolchain is a separate same-project image repository. Keep
    // Dockerfile, vendor inputs, and heph.images out of the site repository so
    // ordinary content commits do not enqueue a new OCI production job.
    fs::remove_file(destination.join("Dockerfile"))?;
    fs::remove_file(destination.join("heph.images.toml"))?;
    fs::remove_file(destination.join("verify-hugo.sh"))?;
    fs::remove_dir_all(destination.join("vendor"))?;
    initialize_git(&destination, "Cooking blog").await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository.id);
    git(&destination, &["remote", "add", "origin", &remote]).await?;
    authenticated_git(
        &destination,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;
    let source_commit = git_output(&destination, &["rev-parse", "HEAD"]).await;
    Ok(PreparedCookingBlog {
        repository_id: repository.id,
        source_commit: source_commit?,
    })
}

pub(crate) async fn create_cooking_blog_toolchain_repository(
    context: &CookingBuildContext<'_>,
    source: &Path,
) -> Result<(), BuildError> {
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("cooking-blog-toolchain-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            agent_runs_enabled: false,
        })
        .await?;
    let destination = context
        .root
        .join(format!("cooking-blog-toolchain-{}", Uuid::new_v4()));
    fs::create_dir_all(destination.join("vendor"))?;
    for name in ["Dockerfile", "heph.images.toml", "verify-hugo.sh"] {
        fs::copy(source.join(name), destination.join(name))?;
    }
    copy_source_tree(&source.join("vendor"), &destination.join("vendor"))?;
    initialize_git(&destination, "Cooking blog Hugo toolchain").await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository.id);
    git(&destination, &["remote", "add", "origin", &remote]).await?;
    authenticated_git(
        &destination,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;
    wait_for_project_image(
        context.pool,
        repository.id,
        "cooking-blog-hugo",
        context.timeout,
    )
    .await
}

pub(crate) async fn wait_for_project_image(
    pool: &PgPool,
    repository_id: RepositoryId,
    key: &str,
    timeout: Duration,
) -> Result<(), BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT status, failure_reason, image_reference
               FROM repository_oci_image_definitions
              WHERE source_repository_id = $1 AND key = $2
              ORDER BY updated_at DESC, id DESC
              LIMIT 1",
        )
        .bind(repository_id.as_uuid())
        .bind(key)
        .fetch_optional(pool)
        .await?;
        match row {
            Some((status, _failure_reason, Some(reference))) if status == "ready" => {
                let digest_pinned = reference
                    .rsplit_once("@sha256:")
                    .is_some_and(|(_, digest)| {
                        digest.len() == 64
                            && digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                    });
                if !digest_pinned {
                    return Err(invalid_state(
                        "project image became ready without a digest-pinned reference",
                    ));
                }
                return Ok(());
            }
            Some((status, failure_reason, _)) if status == "failed" => {
                return Err(invalid_state(&format!(
                    "cooking blog project image failed: {}",
                    failure_reason.unwrap_or_else(|| String::from("unspecified failure")),
                )));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state(
                "timed out waiting for cooking blog project image readiness",
            ));
        }
        sleep(Duration::from_millis(500)).await;
    }
}
