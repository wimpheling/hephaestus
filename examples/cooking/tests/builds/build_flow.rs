// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Builds and publishes the three explicit update-hook variants through the
/// production source/build/release path.  The source copies are test-owned
/// variants under the temporary fixture root; the canonical checkout is never
/// modified.
pub(crate) async fn build_and_publish_update_variants(
    context: CookingBuildContext<'_>,
    base: &PublishedCookingRepository,
) -> Result<PublishedCookingUpdateBuilds, BuildError> {
    // Reuse the canonical agent repository so all candidates remain in its
    // production-created family and satisfy CreateUpdate's family check.
    let migrate_source = base.working_path.clone();
    fs::write(
        migrate_source.join(".cooking-update-migrate"),
        b"canonical v2 migration candidate\n",
    )?;
    let migrate = build_one_in_repository(
        &context,
        "cooking-agent-update-migrate",
        migrate_source,
        "Cooking agent update migration",
        base.repository_id,
        true,
        true,
    )
    .await?;
    let rollback_source = migrate.working_path.clone();
    apply_update_variant(&rollback_source, UpdateVariant::Rollback)?;
    let rollback = build_one_in_repository(
        &context,
        "cooking-agent-update-rollback",
        rollback_source,
        "Cooking agent update rollback",
        base.repository_id,
        true,
        true,
    )
    .await?;
    let abnormal_source = migrate.working_path.clone();
    apply_update_variant(&abnormal_source, UpdateVariant::Abnormal)?;
    let abnormal = build_one_in_repository(
        &context,
        "cooking-agent-update-abnormal",
        abnormal_source,
        "Cooking agent update abnormal",
        base.repository_id,
        true,
        true,
    )
    .await?;
    Ok(PublishedCookingUpdateBuilds {
        migrate,
        rollback,
        abnormal,
    })
}

// Keep source publication and the resulting RPC mutations in one ordered
// sequence so returned provenance cannot describe a partially published pair.
#[allow(clippy::too_many_lines)]
pub(crate) async fn build_one(
    context: &CookingBuildContext<'_>,
    key: &str,
    source: PathBuf,
    display_name: &str,
    expect_update_hook: bool,
) -> Result<PublishedCookingRepository, BuildError> {
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("{key}-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            // This helper creates build-only repositories.  The scenario
            // attaches the resulting release explicitly before running it.
            agent_runs_enabled: false,
        })
        .await?;
    build_one_in_repository(
        context,
        key,
        source,
        display_name,
        repository.id,
        false,
        expect_update_hook,
    )
    .await
}

// Every release in one update sequence must retain the production-created
// family identity.  Building successive commits in one repository gives the
// release worker that family continuity without mutating release metadata.
#[allow(clippy::too_many_lines)]
pub(crate) async fn build_one_in_repository(
    context: &CookingBuildContext<'_>,
    key: &str,
    source: PathBuf,
    display_name: &str,
    repository_id: RepositoryId,
    reuse_checkout: bool,
    expect_update_hook: bool,
) -> Result<PublishedCookingRepository, BuildError> {
    let source_path = if reuse_checkout {
        source
    } else {
        let destination = context
            .root
            .join(format!("cooking-build-{key}-{}", Uuid::new_v4()));
        copy_source_tree(&source, &destination)?;
        initialize_git(&destination, display_name).await?;
        destination
    };
    if reuse_checkout {
        git(&source_path, &["add", "--all"]).await?;
        git(&source_path, &["commit", "--message", display_name]).await?;
    }
    let source_commit = git_output(&source_path, &["rev-parse", "HEAD"]).await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository_id);
    if !reuse_checkout {
        git(&source_path, &["remote", "add", "origin", &remote]).await?;
    }
    authenticated_git(
        &source_path,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;

    let build_request_id =
        wait_for_build_row(context.pool, repository_id, &source_commit, context.timeout).await?;
    let build = wait_for_successful_build(
        context.running,
        context.identity.rpc_token,
        build_request_id,
        context.timeout,
    )
    .await?;
    let release_id = build
        .release_id
        .into_option()
        .ok_or_else(|| invalid_state("successful build has no release"))?
        .value
        .parse::<Uuid>()?;
    let draft_release = wait_for_draft_release(
        context.running,
        context.identity.rpc_token,
        release_id,
        context.timeout,
    )
    .await?;
    let draft_agent = draft_release
        .agents
        .first()
        .ok_or_else(|| invalid_state("GetRelease returned no cooking agent"))?;
    if expect_update_hook && draft_agent.update_hook.as_option().is_none() {
        return Err(invalid_state(
            "GetRelease omitted the canonical cooking update hook",
        ));
    }

    let version = format!("v1.0.0-{}", &source_commit[..12]);
    let release_client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/SetDraftVersion",
    )?;
    let draft = release_client
        .set_draft_version(SetDraftVersionRequest {
            context: mutation_context("set-draft-version").into(),
            release_id: opaque(release_id).into(),
            version: version.clone(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("SetDraftVersion returned no release"))?;
    if draft.state.to_i32() != 1 {
        return Err(invalid_state(
            "SetDraftVersion did not leave a draft release",
        ));
    }
    let release_client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/PublishRelease",
    )?;
    let published = release_client
        .publish_release(PublishReleaseRequest {
            context: mutation_context("publish-release").into(),
            release_id: opaque(release_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("PublishRelease returned no release"))?;
    if published.state.to_i32() != 2 {
        return Err(invalid_state("PublishRelease did not publish the release"));
    }

    let release_agent_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM release_agents
          WHERE release_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(release_id)
    .fetch_one(context.pool)
    .await?;

    let provenance_path = context
        .root
        .join(format!("cooking-build-provenance-{key}-{}", Uuid::new_v4()));
    copy_source_tree(&source_path, &provenance_path)?;
    Ok(PublishedCookingRepository {
        repository_id,
        actor_id: context.identity.actor.user_id,
        source_commit,
        build_request_id,
        release_id,
        release_agent_id,
        version,
        build_definition_hash: build.build_definition_hash,
        configuration_hash: build.configuration_hash,
        manifest_hash: published.manifest_hash,
        source_path: provenance_path,
        working_path: source_path,
    })
}

#[derive(Clone, Copy)]
pub(crate) enum UpdateVariant {
    Rollback,
    Abnormal,
}

pub(crate) fn apply_update_variant(
    source_path: &Path,
    variant: UpdateVariant,
) -> Result<(), BuildError> {
    let config_path = source_path.join("agent.toml");
    let config = fs::read_to_string(&config_path)?;
    let arguments = match variant {
        UpdateVariant::Rollback => "arguments = [\"--rollback-fixture\"]",
        UpdateVariant::Abnormal => "arguments = [\"--abnormal-fixture\"]",
    };
    // Variants are applied sequentially to one production family checkout.
    // Replace whichever prior hook marker is present so the abnormal release
    // cannot accidentally retain the rollback hook's nonzero exit behavior.
    let config = config
        .replace("arguments = [\"--migrate\"]", arguments)
        .replace("arguments = [\"--rollback-fixture\"]", arguments);
    fs::write(config_path, config)?;
    if matches!(variant, UpdateVariant::Rollback) {
        let python_path = source_path.join("cooking_agent.py");
        let source = fs::read_to_string(&python_path)?;
        let source = source.replace(
            "if version == 2:\n            return\n",
            "if version == 2 and fail:\n            raise ValueError('deliberate migration rollback from v2')\n        if version == 2:\n            return\n",
        );
        fs::write(python_path, source)?;
    }
    if matches!(variant, UpdateVariant::Abnormal) {
        let python_path = source_path.join("cooking_agent.py");
        let source = fs::read_to_string(&python_path)?;
        let source = source
            .replace("import argparse\n", "import argparse\nimport os\nimport signal\n")
            .replace(
                "parser.add_argument('--rollback-fixture', action='store_true')",
                "parser.add_argument('--rollback-fixture', action='store_true')\n    parser.add_argument('--abnormal-fixture', action='store_true')",
            )
            .replace(
                "if args.migrate or args.rollback_fixture:\n",
                "if args.abnormal_fixture:\n        os.kill(os.getpid(), signal.SIGKILL)\n    if args.migrate or args.rollback_fixture:\n",
            );
        fs::write(python_path, source)?;
    }
    Ok(())
}
