//! Controlled startup/stop concurrency; mocked exit is not native reap proof.
use super::support::*;

struct BlockedStartup {
    worker: MockWorker,
    cancel_entered: Notify,
    cancels: AtomicUsize,
    kills: AtomicUsize,
}

impl BlockedStartup {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            worker: MockWorker::blocking_start(),
            cancel_entered: Notify::new(),
            cancels: AtomicUsize::new(0),
            kills: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl WorkerBackend for BlockedStartup {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        if matches!(command, WorkerCommand::Cancel { .. }) {
            self.cancels.fetch_add(1, Ordering::Relaxed);
            self.cancel_entered.notify_one();
            return std::future::pending().await;
        }
        self.worker.request(command).await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        self.worker.subscribe_events()
    }

    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.worker.subscribe_process_exit()
    }

    async fn kill(&self) -> Result<(), VmError> {
        self.kills.fetch_add(1, Ordering::Relaxed);
        self.worker.kill().await
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        self.worker.wait_process().await
    }
}

fn start_task(vm: &Arc<LibkrunInstance>) -> tokio::task::JoinHandle<Result<(), VmError>> {
    let vm = Arc::clone(vm);
    tokio::spawn(async move { vm.start().await })
}

#[tokio::test(start_paused = true)]
async fn force_stop_interrupts_blocked_start_without_releasing_id() {
    let temp = TempDir::new().unwrap();
    let worker = BlockedStartup::new();
    let vm = instance_with_options(&temp, worker.clone(), Duration::from_secs(60), None);
    let started = start_task(&vm);
    worker.worker.start_entered.notified().await;
    tokio::time::timeout(Duration::from_millis(1), vm.stop(StopMode::Force))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        started.await.unwrap(),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(worker.cancels.load(Ordering::Relaxed), 0);
    assert_eq!(worker.worker.start_calls.load(Ordering::Relaxed), 1);
    assert_eq!(vm.wait().await.unwrap().signal, Some(9));
    assert!(!vm.cleanup_complete.load(Ordering::Acquire));
    assert!(vm.provider_ids.ids.lock().await.contains(vm.id()));
    vm.destroy().await.unwrap();
    drop(vm);
}

#[tokio::test(start_paused = true)]
async fn graceful_stop_bounds_blocked_cancel_during_startup() {
    let temp = TempDir::new().unwrap();
    let worker = BlockedStartup::new();
    let vm = instance_with_options(&temp, worker.clone(), Duration::from_secs(60), None);
    let started = start_task(&vm);
    worker.worker.start_entered.notified().await;
    let stopping = tokio::spawn({
        let vm = Arc::clone(&vm);
        async move {
            vm.stop(StopMode::Graceful {
                timeout: Duration::from_millis(5),
            })
            .await
        }
    });
    worker.cancel_entered.notified().await;
    assert!(!stopping.is_finished());
    assert!(worker.worker.process_exit.borrow().is_none());
    tokio::time::advance(Duration::from_millis(5)).await;
    stopping.await.unwrap().unwrap();
    assert!(matches!(
        started.await.unwrap(),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(worker.cancels.load(Ordering::Relaxed), 1);
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert_eq!(vm.wait().await.unwrap().signal, Some(9));
    assert!(!vm.cleanup_complete.load(Ordering::Acquire));
    assert!(vm.provider_ids.ids.lock().await.contains(vm.id()));
    vm.destroy().await.unwrap();
    drop(vm);
}

#[tokio::test(start_paused = true)]
async fn destroy_preempts_blocked_graceful_stop_after_start_interruption() {
    let temp = TempDir::new().unwrap();
    let worker = BlockedStartup::new();
    let vm = instance_with_options(&temp, worker.clone(), Duration::from_secs(60), None);
    let started = start_task(&vm);
    worker.worker.start_entered.notified().await;
    let stopping = tokio::spawn({
        let vm = Arc::clone(&vm);
        async move {
            vm.stop(StopMode::Graceful {
                timeout: Duration::from_secs(30),
            })
            .await
        }
    });
    worker.cancel_entered.notified().await;
    assert!(worker.worker.process_exit.borrow().is_none());
    tokio::time::timeout(Duration::from_millis(1), vm.destroy())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(stopping.await.unwrap(), Err(VmError::Destroyed)));
    assert!(matches!(
        started.await.unwrap(),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    assert!(worker.worker.process_exit.borrow().is_some());
    assert!(vm.cleanup_complete.load(Ordering::Acquire));
    assert!(!vm.provider_ids.ids.lock().await.contains(vm.id()));
    assert!(matches!(vm.start().await, Err(VmError::Destroyed)));
    vm.destroy().await.unwrap();
    assert_eq!(worker.kills.load(Ordering::Relaxed), 1);
    drop(vm);
}

#[tokio::test]
async fn stop_before_start_remains_noop_and_does_not_poison_start() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let vm = instance(&temp, worker.clone());
    vm.stop(StopMode::Force).await.unwrap();
    assert!(!*vm.stop_requested.borrow());
    assert!(worker.process_exit.borrow().is_none());
    vm.start().await.unwrap();
    assert_eq!(worker.start_calls.load(Ordering::Relaxed), 1);
    vm.destroy().await.unwrap();
    drop(vm);
}
