use super::*;

/// Builds the canonical, adversarial, update, and blog cooking releases together.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn prepare_cooking_releases(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project_id: ProjectId,
    user_id: UserId,
    repositories: &PgForgeRepository,
    identity: &AuthenticatedIdentity,
    token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    timeout: Duration,
    workload_phase_timing: bool,
) -> (
    cooking_builds::PublishedCookingBuilds,
    cooking_builds::PublishedCookingRepository,
    cooking_builds::PublishedCookingRepository,
    Option<cooking_builds::PublishedCookingUpdateBuilds>,
    cooking_builds::PreparedCookingBlog,
) {
    let (prepared_releases, blog_repository) = Box::pin(join_cooking_preparation(
        async {
            let builds = {
                let production_project_build_timer =
                    WorkloadPhaseTimer::start("production-project-build", workload_phase_timing);
                let result =
                    cooking_builds::build_and_publish(cooking_builds::CookingBuildContext {
                        pool,
                        running,
                        root,
                        source_root,
                        project_id,
                        repositories,
                        identity: cooking_builds::CookingIdentity {
                            actor: identity,
                            git_token: token,
                            rpc_token,
                        },
                        timeout,
                    })
                    .await;
                production_project_build_timer.finish(result.is_ok());
                result.expect("real cooking source build and publish proof")
            };
            let adversarial_agent_build = cooking_builds::build_and_publish_adversarial_agent(
                &cooking_builds::CookingBuildContext {
                    pool,
                    running,
                    root,
                    source_root,
                    project_id,
                    repositories,
                    identity: cooking_builds::CookingIdentity {
                        actor: identity,
                        git_token: token,
                        rpc_token,
                    },
                    timeout,
                },
                &builds.agent,
            )
            .await
            .expect("publish adversarial cooking agent destination release");
            assert_ne!(
                adversarial_agent_build.release_id, builds.agent.release_id,
                "the adversarial agent must use a distinct published release"
            );
            let adversarial_gateway_build = cooking_builds::build_and_publish_adversarial_gateway(
                &cooking_builds::CookingBuildContext {
                    pool,
                    running,
                    root,
                    source_root,
                    project_id,
                    repositories,
                    identity: cooking_builds::CookingIdentity {
                        actor: identity,
                        git_token: token,
                        rpc_token,
                    },
                    timeout,
                },
                &builds.gateway,
            )
            .await
            .expect("publish adversarial foreign-slot gateway release");
            assert_eq!(
                adversarial_gateway_build.repository_id, builds.gateway.repository_id,
                "the adversarial release must remain in the canonical release family"
            );
            assert_ne!(
                adversarial_gateway_build.release_id, builds.gateway.release_id,
                "the adversarial probe must use a distinct published release"
            );
            for published in [&builds.gateway, &builds.agent, &adversarial_agent_build] {
                assert_eq!(published.actor_id, user_id);
                assert!(!published.repository_id.as_uuid().is_nil());
                assert!(!published.source_commit.is_empty());
                assert!(!published.build_request_id.is_nil());
                assert!(!published.release_id.is_nil());
                assert!(!published.release_agent_id.is_nil());
                assert!(!published.version.is_empty());
                assert!(!published.build_definition_hash.is_empty());
                assert!(!published.configuration_hash.is_empty());
                assert!(!published.manifest_hash.is_empty());
                assert!(published.source_path.is_dir());
                assert!(published.working_path.is_dir());
            }
            let update_builds = if env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1") {
                Some(
                    cooking_builds::build_and_publish_update_variants(
                        cooking_builds::CookingBuildContext {
                            pool,
                            running,
                            root,
                            source_root,
                            project_id,
                            repositories,
                            identity: cooking_builds::CookingIdentity {
                                actor: identity,
                                git_token: token,
                                rpc_token,
                            },
                            timeout,
                        },
                        &builds.agent,
                    )
                    .await
                    .expect("publish cooking update-hook variants"),
                )
            } else {
                None
            };
            (
                builds,
                adversarial_agent_build,
                adversarial_gateway_build,
                update_builds,
            )
        },
        async {
            let blog_repository = cooking_builds::create_cooking_blog_repository(
                &cooking_builds::CookingBuildContext {
                    pool,
                    running,
                    root,
                    source_root,
                    project_id,
                    repositories,
                    identity: cooking_builds::CookingIdentity {
                        actor: identity,
                        git_token: token,
                        rpc_token,
                    },
                    timeout,
                },
            )
            .await
            .expect("create and push separate cooking blog repository");
            assert!(!blog_repository.source_commit.is_empty());
            blog_repository
        },
    ))
    .await;
    let (builds, adversarial_agent_build, adversarial_gateway_build, update_builds) =
        prepared_releases;
    (
        builds,
        adversarial_agent_build,
        adversarial_gateway_build,
        update_builds,
        blog_repository,
    )
}
