use super::*;

#[path = "runtime_setup.rs"]
mod runtime_setup;
use runtime_setup::*;
#[path = "setup_phases.rs"]
mod setup_phases;
use setup_phases::*;
#[path = "runtime_flow.rs"]
mod runtime_flow;
use runtime_flow::*;
#[path = "results.rs"]
mod results;
use results::*;

struct SmartHttpContext {
    pool: PgPool,
    temporary: tempfile::TempDir,
    storage: Arc<GitStorage>,
    repository_service: Arc<PgForgeRepository>,
    project: forge_domain::Project,
    repository: forge_domain::Repository,
    other_repository: forge_domain::Repository,
    user_id: UserId,
    authorizer: Arc<RecordingAuthorizer>,
    backend: PathBuf,
    source: PathBuf,
    commit: String,
    runtime_commit: String,
    server: tokio::task::JoinHandle<()>,
}

async fn setup(pool: PgPool) -> SmartHttpContext {
    let (temporary, storage, repository_service) = initialize(pool.clone()).await;
    let (user_id, identity, project, repository, other_repository) =
        seed_repository_graph(&pool, &repository_service).await;
    let (authorizer, basic_credential, backend, remote, server) =
        start_http(&pool, &storage, &repository_service, &repository, &identity).await;
    let (source, commit, runtime_commit) = seed_git(
        &pool,
        &temporary,
        &project,
        &repository,
        user_id,
        &basic_credential,
        &remote,
    )
    .await;
    SmartHttpContext {
        pool,
        temporary,
        storage,
        repository_service,
        project,
        repository,
        other_repository,
        user_id,
        authorizer,
        backend,
        source,
        commit,
        runtime_commit,
        server,
    }
}

#[tokio::test]
#[serial]
async fn clone_fetch_push_audit_and_run_request() {
    let Some(pool) = postgres_pool().await else {
        eprintln!("skipping: HEPHAESTUS_POSTGRES_TEST_URL is not set");
        return;
    };
    let context = setup(pool).await;
    let runtime = prepare_runtime(&context).await;
    exercise_runtime(&context, &runtime).await;
    assert_results(&context).await;
    context.server.abort();
}
