use super::{BuildError, CookingBuildContext, invalid_state, opaque, rpc_build_client};
use rpc_proto::messages::hephaestus::build::v1::{BuildState, GetBuildRequest};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

pub(crate) async fn wait_for_blog_configuration(
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

pub(crate) async fn wait_for_build(
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
