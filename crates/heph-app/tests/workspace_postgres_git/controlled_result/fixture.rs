use super::*;

pub struct ControlledFixture {
    pub pool: PgPool,
    pub _temporary: tempfile::TempDir,
    pub root: PathBuf,
    pub _repository_root: PathBuf,
    pub forge: PgForgeRepository,
    pub _project: forge_domain::Project,
    pub repository: forge_domain::Repository,
    pub _work: PathBuf,
    pub accepted: CommitSha,
    pub newer: CommitSha,
    pub bare: PathBuf,
    pub runs: PgRunRepository,
    pub _workspace_repository: Arc<PgWorkspaceMetadataRepository>,
    pub manager: LocalWorkspaceManager,
    pub _mailbox_run: run_domain::Run,
    pub run: run_domain::Run,
}

// This fixture keeps the complete PostgreSQL, Git, and workspace graph together
// so the controlled-result test exercises the real cross-adapter setup.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn setup(pool: PgPool) -> ControlledFixture {
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let repository_root = root.join("repositories");
    let storage = Arc::new(
        GitStorage::initialize(&repository_root)
            .await
            .expect("initialize repositories"),
    );
    let forge = PgForgeRepository::new(pool.clone(), storage);
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind(format!("workspace-{organization_id}"))
        .execute(&pool)
        .await
        .expect("organization");
    let project = forge
        .create_project_trusted(organization_id, "workspace-project")
        .await
        .expect("project");
    let repository = forge
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("workspace-repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");

    let work = root.join("input");
    tokio::fs::create_dir(&work).await.expect("input work tree");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Workspace Test"]).await;
    git(
        &work,
        &["config", "user.email", "workspace@example.invalid"],
    )
    .await;
    tokio::fs::write(work.join("agent.toml"), agent_config(repository.id))
        .await
        .expect("agent config");
    tokio::fs::write(work.join("input.txt"), "accepted\n")
        .await
        .expect("input file");
    tokio::fs::create_dir(work.join("reports"))
        .await
        .expect("report directory");
    tokio::fs::write(work.join("reports/result.txt"), "initial\n")
        .await
        .expect("initial declared file");
    git(&work, &["add", "."]).await;
    git(&work, &["commit", "-m", "accepted input"]).await;
    let accepted =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("accepted commit");
    seed_attached_instance(&pool, project.id, repository.id, accepted.as_str()).await;
    let bare = repository_root.join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let receive = forge
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "workspace-test",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("main ref"),
                old_commit: None,
                new_commit: Some(accepted.clone()),
            }],
        )
        .await
        .expect("accept receive");
    let request = receive.run_requests.first().expect("one run request");
    let command: StartRun = request.command.clone();
    let runs = PgRunRepository::new(pool.clone());
    let run = runs
        .create_run(&command)
        .await
        .expect("create durable run")
        .run;

    tokio::fs::write(work.join("input.txt"), "newer branch tip\n")
        .await
        .expect("newer input");
    git(&work, &["add", "input.txt"]).await;
    git(&work, &["commit", "-m", "move main"]).await;
    let newer =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("newer commit");
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;

    let workspace_repository = Arc::new(PgWorkspaceMetadataRepository::new(pool.clone()));
    let mailbox_run = assert_mailbox_target_survives_ref_movement(
        &pool,
        &command,
        &accepted,
        &newer,
        project.id.as_uuid(),
        repository.id.as_uuid(),
    )
    .await;
    let mut manager = LocalWorkspaceManager::new(
        Arc::clone(&workspace_repository) as Arc<dyn WorkspaceMetadataRepository>,
        Arc::clone(&workspace_repository) as Arc<dyn ResultRepository>,
        LocalWorkspaceConfig {
            workspace_root: root.join("workspaces"),
            artifact_root: root.join("artifacts"),
            repository_root: repository_root.clone(),
            git_binary: git_binary().await,
            limits: WorkspaceLimits::default(),
        },
    )
    .expect("workspace configuration");
    manager.initialize().expect("workspace roots");
    assert_mailbox_result_proposal(&pool, &manager, &mailbox_run, &accepted).await;
    ControlledFixture {
        pool,
        _temporary: temporary,
        root,
        _repository_root: repository_root,
        forge,
        _project: project,
        repository,
        _work: work,
        accepted,
        newer,
        bare,
        runs,
        _workspace_repository: workspace_repository,
        manager,
        _mailbox_run: mailbox_run,
        run,
    }
}
