use super::*;

pub(super) struct LifecycleLogStore {
    pub(super) append_started: Notify,
    pub(super) release: Notify,
    pub(super) blocked: AtomicBool,
    pub(super) calls: AtomicUsize,
    pub(super) records: AtomicUsize,
    pub(super) responses: Mutex<
        VecDeque<Result<crate::GatewayServiceLogAppendOutcome, crate::GatewayServiceLogStoreError>>,
    >,
    pub(super) leases: Mutex<Vec<(GatewayServiceIdentity, i64)>>,
    pub(super) events: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl GatewayServiceLogStore for LifecycleLogStore {
    async fn append_batch(
        &self,
        lease: &GatewayServiceInstanceLease,
        _: &GatewayServiceOwner,
        batch: crate::GatewayServiceLogAppendBatch,
    ) -> Result<crate::GatewayServiceLogAppendOutcome, crate::GatewayServiceLogStoreError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.records
            .fetch_add(batch.records.len(), Ordering::Relaxed);
        self.leases
            .lock()
            .expect("log store leases")
            .push((lease.identity, lease.fencing_token));
        self.append_started.notify_one();
        if self.blocked.load(Ordering::Relaxed) {
            self.release.notified().await;
        }
        self.events
            .lock()
            .expect("log store events")
            .push("log-flush");
        self.responses
            .lock()
            .expect("log store responses")
            .pop_front()
            .unwrap_or_else(|| Ok(crate::GatewayServiceLogAppendOutcome::default()))
    }
}

#[tokio::test]
async fn lifecycle_writer_does_not_starve_worker_and_flushes_after_teardown() {
    let identity = identity();
    let owner = GatewayServiceOwner::new("writer-test-host", Uuid::new_v4()).expect("owner");
    let lease = lease(&owner, identity);
    let store = Arc::new(LifecycleLogStore {
        append_started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        responses: Mutex::new(VecDeque::new()),
        leases: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let store_port: Arc<dyn GatewayServiceLogStore> = store.clone();
    let buffer = crate::ServiceLogBufferHandle::new();
    buffer.try_record(
        vm_trait::LogStream::Stdout,
        ::time::OffsetDateTime::UNIX_EPOCH,
        b"ready-service-log",
    );
    let writer = ServiceLogWriter::new(
        buffer,
        store_port,
        lease,
        owner,
        ServiceLogWriterPolicy::default(),
    )
    .expect("writer");
    let worker_events = Arc::clone(&store);
    let worker: WorkerFuture = Box::pin(async move {
        worker_events.append_started.notified().await;
        worker_events
            .events
            .lock()
            .expect("worker events")
            .push("destroyed");
        worker_events.blocked.store(false, Ordering::Relaxed);
        worker_events.release.notify_waiters();
        Ok(())
    });

    timeout(
        Duration::from_secs(1),
        drive_worker_with_logs(worker, writer),
    )
    .await
    .expect("worker and bounded log flush")
    .expect("worker result");
    assert!(store.calls.load(Ordering::Relaxed) >= 1);
    assert!(store.records.load(Ordering::Relaxed) >= 1);
    assert_eq!(
        store
            .events
            .lock()
            .expect("log store events")
            .first()
            .copied(),
        Some("destroyed")
    );
    assert!(
        store
            .events
            .lock()
            .expect("log store events")
            .iter()
            .all(|event| *event == "destroyed" || *event == "log-flush")
    );
}
