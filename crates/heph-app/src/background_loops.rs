use super::{
    AppError, Arc, AtomicBool, CancellationToken, Duration, Future, OciBuilderWorkers, Ordering,
    PgPool, RunOrchestrator, component, event_adapter, runtime_git_listener,
};
use async_trait::async_trait;
use axum::Router;
use control_plane_postgres::connect as connect_control_plane;
use event_postgres::ReleaseOutboxPublisher;
use forge_postgres::PgForgeRepository;
use forge_service::ForgeNatsOutboxPublisher;
use heph_run::{CancelRun, RunSecretManager};
use heph_secret::EphemeralSecretConfig;
use mailbox_dispatch::MailboxDispatchStore;
use mailbox_dispatch::MailboxOutboxPublisher;
use oci_builder_worker::OciWorkerError;
use review_service::ReviewOutboxPublisher;
use runtime_types::{CommandId, RunId};
use secret_postgres::{SecretRuntimeService, SecretService, initialize_manager};
use secret_runtime::FilesystemSecretMountProvider;
use secret_store::{EncryptedStore, LocalKeyProvider};
use tokio::{sync::oneshot, task::JoinHandle};
pub async fn reap_failed_start(tasks: Vec<JoinHandle<Result<(), String>>>) {
    for task in tasks {
        task.abort();
        drop(task.await);
    }
}

pub fn spawn_runtime_git_listener(
    listener: runtime_git_listener::RuntimeGitListener,
    router: Router,
    cancellation: &CancellationToken,
    tasks: &mut Vec<JoinHandle<Result<(), String>>>,
) -> oneshot::Receiver<()> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let runtime_git_cancel = cancellation.clone();
    tasks.push(tokio::spawn(async move {
        if ready_tx.send(()).is_err() {
            return Ok(());
        }
        let result = listener
            .serve(router, runtime_git_cancel.clone())
            .await
            .map_err(|error| error.to_string());
        if result.is_err() && !runtime_git_cancel.is_cancelled() {
            runtime_git_cancel.cancel();
        }
        result
    }));
    ready_rx
}

pub async fn oci_builder_loop(workers: Arc<OciBuilderWorkers>, cancellation: CancellationToken) {
    let mut interval = tokio::time::interval(workers.poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = interval.tick() => {
                oci_builder_pass(&workers).await;
            }
        }
    }
}

// The two worker outcomes and their independently durable manifest update are
// intentionally explicit; Clippy counts the async/logging expansion as well.
#[allow(clippy::cognitive_complexity)]
pub async fn oci_builder_pass(workers: &OciBuilderWorkers) {
    if let Err(error) = workers.preparation.run_once().await {
        tracing::warn!(%error, "OCI preparation worker pass failed");
    }
    let materialization_changed = match workers.materialization.run_once().await {
        Ok(changed) => changed,
        Err(error) => {
            tracing::warn!(%error, "OCI rootfs materialization worker pass failed");
            false
        }
    };
    if let Err(error) =
        write_oci_manifest_if_dirty(&workers.manifest_dirty, materialization_changed, || {
            workers.materialization.write_manifest(&workers.manifest)
        })
        .await
    {
        tracing::warn!(%error, "OCI builder root manifest update failed");
    }
    // Refresh from durable roots on every pass. A successful materialization
    // followed by a transient manifest or cache error must be retried even
    // when the next claim pass has no new materialization job.
    if let Err(error) = workers.refresh_image_filesystems().await {
        tracing::warn!(%error, "OCI builder image cache refresh failed");
    }
}

pub async fn write_oci_manifest_if_dirty<F, Fut>(
    dirty: &AtomicBool,
    materialization_changed: bool,
    write_manifest: F,
) -> Result<(), OciWorkerError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), OciWorkerError>>,
{
    if materialization_changed {
        dirty.store(true, Ordering::Release);
    }
    if !dirty.load(Ordering::Acquire) {
        return Ok(());
    }
    write_manifest().await?;
    dirty.store(false, Ordering::Release);
    Ok(())
}

pub struct OutboxWorker {
    pub forge_publisher: ForgeNatsOutboxPublisher,
    pub release_publisher: ReleaseOutboxPublisher,
    pub review_publisher: ReviewOutboxPublisher,
    pub event_publisher: event_adapter::EventPublisher,
    pub mailbox_publisher: MailboxOutboxPublisher,
    pub forge: Arc<PgForgeRepository>,
    pub poll_interval: Duration,
    pub batch_size: i64,
}

impl OutboxWorker {
    // Rust 1.85 Clippy incorrectly reports Tokio's private select expansion as
    // redundant public crate visibility.
    #[allow(clippy::redundant_pub_crate)]
    // The explicit worker fan-out keeps each durable outbox and its failure
    // policy visible in one supervised loop.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(self, cancellation: CancellationToken, ready: oneshot::Sender<()>) {
        let mut interval = tokio::time::interval(self.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        if ready.send(()).is_err() {
            return;
        }
        loop {
            tokio::select! {
                () = cancellation.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(error) = self
                        .forge_publisher
                        .publish_pending(self.forge.as_ref(), self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "forge outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .release_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "release outbox publication pass failed");
                    }
                    if let Err(error) = self
                        .review_publisher
                        .publish_pending(self.batch_size)
                        .await
                    {
                        tracing::warn!(%error, "review outbox publication pass failed");
                    }
                    if let Err(error) = self.event_publisher.publish_pending(self.batch_size).await {
                        tracing::warn!(%error, "product-event outbox publication pass failed");
                    }
                    if let Err(error) = self.mailbox_publisher.publish_pending(self.batch_size).await {
                        tracing::warn!(%error, "mailbox outbox publication pass failed");
                    }
                }
            }
        }
    }
}

pub async fn secret_revocation_loop(
    pool: PgPool,
    orchestrator: Arc<RunOrchestrator>,
    poll_interval: Duration,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(poll_interval) => {
                reconcile_revoked_raw_runs(&pool, orchestrator.as_ref()).await?;
            }
        }
    }
}

pub async fn mailbox_recovery_loop(
    store: Arc<dyn MailboxDispatchStore>,
    poll_interval: Duration,
    cancellation: CancellationToken,
    ready: oneshot::Sender<()>,
) -> Result<(), String> {
    store.recover().await.map_err(|error| error.to_string())?;
    store
        .cleanup_expired_payloads(100)
        .await
        .map_err(|error| error.to_string())?;
    if ready.send(()).is_err() {
        return Ok(());
    }
    loop {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            () = tokio::time::sleep(poll_interval) => {
                if let Err(error) = store.recover().await {
                    tracing::warn!(%error, "mailbox recovery pass failed");
                }
                if let Err(error) = store.cleanup_expired_payloads(100).await {
                    tracing::warn!(%error, "mailbox payload retention pass failed");
                }
            }
        }
    }
}

async fn reconcile_revoked_raw_runs(
    pool: &PgPool,
    canceller: &(impl RevokedRawRunCanceller + ?Sized),
) -> Result<usize, String> {
    let run_ids = control_plane_postgres::revoked_raw_run_ids(pool)
        .await
        .map_err(|error| error.to_string())?;
    let mut cancellation_count = 0;
    for run_id in run_ids {
        let run_id = RunId::from_uuid(run_id);
        if canceller.cancel_revoked_raw_run(run_id).await? {
            cancellation_count += 1;
        }
    }
    Ok(cancellation_count)
}

#[async_trait]
trait RevokedRawRunCanceller: Sync {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String>;
}

#[async_trait]
impl RevokedRawRunCanceller for RunOrchestrator {
    async fn cancel_revoked_raw_run(&self, run_id: RunId) -> Result<bool, String> {
        self.cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id,
            reason: String::from("raw secret authority was revoked"),
        })
        .await
        .map_err(|error| error.to_string())
    }
}

pub async fn build_secret_mount_manager(
    pool: PgPool,
    database_url: &str,
    keys: LocalKeyProvider,
    config: EphemeralSecretConfig,
) -> Result<
    (
        Arc<dyn RunSecretManager>,
        Arc<SecretRuntimeService<LocalKeyProvider>>,
        Arc<SecretService<LocalKeyProvider>>,
    ),
    AppError,
> {
    let resolver_pool = connect_control_plane(database_url, 4)
        .await
        .map_err(component("secret resolver PostgreSQL connection"))?;
    let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
    let dispatch = Arc::new(SecretService::new(
        pool.clone(),
        EncryptedStore::new(keys.clone()),
        authorizer.clone(),
    ));
    let runtime = Arc::new(
        SecretRuntimeService::new(
            pool.clone(),
            resolver_pool,
            EncryptedStore::new(keys),
            authorizer,
        )
        .with_mount_provider(Arc::new(FilesystemSecretMountProvider::new())),
    );
    let manager = initialize_manager(
        pool,
        dispatch.as_ref().clone(),
        runtime.as_ref().clone(),
        config,
    )
    .map_err(component("secret mount initialization"))?;
    Ok((Arc::new(manager), runtime, dispatch))
}
