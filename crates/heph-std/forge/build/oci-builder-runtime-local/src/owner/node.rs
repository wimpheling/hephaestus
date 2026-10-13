use super::{
    OciWorkerError, lock,
    models::{Outcome, Phase},
    pin::Pin,
    task::Task,
};
use oci_builder_worker::IsolatedOciBuild;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use vm_trait::{VmInstance, VmProvisionAttempt, VmSpec};
pub(super) type CleanupTask = Arc<Task<Result<(), Arc<OciWorkerError>>>>;
type PreparationTask = Arc<Task<Result<Arc<crate::constants::ScratchDisk>, Arc<OciWorkerError>>>>;

pub(super) enum Leaf {
    Untouched,
    Acquiring {
        _original_path: PathBuf,
        _original_file: Option<Arc<std::fs::File>>,
    },
    Original(Arc<Pin>),
}
pub(super) struct VmBirth {
    pub(super) spec: VmSpec,
    pub(super) attempt: Arc<dyn VmProvisionAttempt>,
    pub(super) instance: Option<Arc<dyn VmInstance>>,
    pub(super) detached: bool,
    pub(super) cleanup: Option<CleanupTask>,
    pub(super) cleanup_history: Vec<CleanupTask>,
}
pub(super) struct State {
    pub(super) closing: bool,
    pub(super) leaves: [Leaf; 3],
    pub(super) mkfs: Option<Arc<super::mkfs::Original>>,
    pub(super) preparation: Option<PreparationTask>,
    pub(super) births: BTreeMap<Phase, VmBirth>,
    pub(super) outcome: Option<Outcome>,
    pub(super) cleanup_error: Option<Arc<OciWorkerError>>,
    pub(super) errors: Vec<Arc<dyn std::error::Error + Send + Sync>>,
}
pub(super) struct Node {
    pub(super) request: IsolatedOciBuild,
    pub(super) paths: [PathBuf; 3],
    pub(super) state: Mutex<State>,
    pub(super) changed: tokio::sync::Notify,
    pub(super) work: Mutex<Option<Arc<Task<Outcome>>>>,
    pub(super) watcher: Mutex<Option<Arc<Task<()>>>>,
    pub(super) cleanup: Mutex<Option<Arc<Task<()>>>>,
    pub(super) mkfs_cleanup: Mutex<Vec<CleanupTask>>,
}
impl Node {
    pub(super) fn new(request: IsolatedOciBuild, paths: [PathBuf; 3]) -> Arc<Self> {
        Arc::new(Self {
            request,
            paths,
            state: Mutex::new(State {
                closing: false,
                leaves: [Leaf::Untouched, Leaf::Untouched, Leaf::Untouched],
                mkfs: None,
                preparation: None,
                births: BTreeMap::new(),
                outcome: None,
                cleanup_error: None,
                errors: Vec::new(),
            }),
            changed: tokio::sync::Notify::new(),
            work: Mutex::new(None),
            watcher: Mutex::new(None),
            cleanup: Mutex::new(None),
            mkfs_cleanup: Mutex::new(Vec::new()),
        })
    }
    pub(super) fn error(&self, error: Arc<dyn std::error::Error + Send + Sync>) {
        lock(&self.state).errors.push(error);
    }
    pub(super) fn close(&self) {
        lock(&self.state).closing = true;
        self.changed.notify_waiters();
    }
    pub(super) fn open(&self) -> Result<(), OciWorkerError> {
        if lock(&self.state).closing {
            Err(OciWorkerError::InvalidConfiguration)
        } else {
            Ok(())
        }
    }
    pub(super) fn acquire(&self, index: usize) -> Result<(), OciWorkerError> {
        let mut state = lock(&self.state);
        if state.closing || !matches!(state.leaves[index], Leaf::Untouched) {
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        state.leaves[index] = Leaf::Acquiring {
            _original_path: self.paths[index].clone(),
            _original_file: None,
        };
        drop(state);
        Ok(())
    }
    pub(super) fn capture_fd(&self, index: usize, file: std::fs::File) -> Arc<std::fs::File> {
        let file = Arc::new(file);
        let mut state = lock(&self.state);
        if let Leaf::Acquiring {
            _original_file: original_file,
            ..
        } = &mut state.leaves[index]
        {
            *original_file = Some(file.clone());
        }
        drop(state);
        file
    }
    pub(super) fn require_detached(&self) -> Result<(), OciWorkerError> {
        if lock(&self.state).births.values().all(|birth| {
            birth.detached
                && birth
                    .instance
                    .as_ref()
                    .is_none_or(|instance| instance.id() == &birth.spec.id)
        }) {
            Ok(())
        } else {
            Err(super::errors::failed("original VM binding"))
        }
    }
    pub(super) fn leaf(&self, index: usize, pin: Arc<Pin>) {
        lock(&self.state).leaves[index] = Leaf::Original(pin);
        self.changed.notify_waiters();
    }
    pub(super) fn pin(&self, index: usize) -> Result<Arc<Pin>, OciWorkerError> {
        match &lock(&self.state).leaves[index] {
            Leaf::Original(pin) => Ok(pin.clone()),
            _ => Err(OciWorkerError::UnsafeMaterializationPath),
        }
    }
    pub(super) fn mkfs(&self, original: Arc<super::mkfs::Original>) -> Result<(), OciWorkerError> {
        let mut state = lock(&self.state);
        if state.closing || state.mkfs.is_some() {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        state.mkfs = Some(original);
        self.changed.notify_waiters();
        drop(state);
        Ok(())
    }
    pub(super) fn install(
        &self,
        phase: Phase,
        spec: VmSpec,
        attempt: Arc<dyn VmProvisionAttempt>,
    ) -> Result<(), OciWorkerError> {
        let mut state = lock(&self.state);
        if state.closing || state.births.contains_key(&phase) {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        state.births.insert(
            phase,
            VmBirth {
                spec,
                attempt,
                instance: None,
                detached: false,
                cleanup: None,
                cleanup_history: Vec::new(),
            },
        );
        self.changed.notify_waiters();
        drop(state);
        Ok(())
    }
    pub(super) fn instance(
        &self,
        phase: Phase,
        instance: Arc<dyn VmInstance>,
    ) -> Result<(), OciWorkerError> {
        let mut state = lock(&self.state);
        let birth = state
            .births
            .get_mut(&phase)
            .ok_or(OciWorkerError::InvalidConfiguration)?;
        if birth.instance.is_some() {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let matches = instance.id() == &birth.spec.id;
        birth.instance = Some(instance);
        let result = if matches {
            Ok(())
        } else {
            Err(OciWorkerError::InvalidConfiguration)
        };
        drop(state);
        result
    }
    // Capture every independent original cleanup before any resource await.
    // A current unjoined handle is reused; historical joined handles stay saved.
    pub(super) fn start_detach(self: &Arc<Self>) -> Vec<CleanupTask> {
        let mut state = lock(&self.state);
        let mut tasks = Vec::new();
        for (phase, birth) in &mut state.births {
            if birth.detached {
                if let Some(current) = &birth.cleanup {
                    tasks.push(current.clone());
                }
                continue;
            }
            if let Some(current) = &birth.cleanup
                && !current.joined()
            {
                tasks.push(current.clone());
                continue;
            }
            let phase = *phase;
            let attempt = birth.attempt.clone();
            let owner = self.clone();
            let (release, registered) = tokio::sync::oneshot::channel();
            let task = Task::new(tokio::spawn(async move {
                if let Err(error) = registered.await {
                    owner.error(Arc::new(error));
                    return Err(Arc::new(OciWorkerError::InvalidConfiguration));
                }
                match attempt.cleanup().await {
                    Ok(()) => {
                        let mut state = lock(&owner.state);
                        let Some(birth) = state.births.get_mut(&phase) else {
                            return Err(Arc::new(OciWorkerError::InvalidConfiguration));
                        };
                        if !Arc::ptr_eq(&birth.attempt, &attempt) {
                            return Err(Arc::new(OciWorkerError::InvalidConfiguration));
                        }
                        birth.detached = true;
                        drop(state);
                        owner.changed.notify_waiters();
                        Ok(())
                    }
                    Err(error) => {
                        owner.error(Arc::new(error));
                        Err(Arc::new(super::errors::failed("original detach")))
                    }
                }
            }));
            birth.cleanup = Some(task.clone());
            birth.cleanup_history.push(task.clone());
            tasks.push(task);
            let _registered = release.send(());
        }
        drop(state);
        tasks
    }
    pub(super) async fn join_detach(
        self: &Arc<Self>,
        tasks: Vec<CleanupTask>,
    ) -> Result<(), OciWorkerError> {
        let mut failure = None;
        for task in tasks {
            let result = task.join(self).await.map_or_else(
                |_| Err(super::errors::failed("original detach task")),
                |result| result.map_err(super::errors::observed),
            );
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub(super) async fn detach(self: &Arc<Self>) -> Result<(), OciWorkerError> {
        self.join_detach(self.start_detach()).await
    }
    #[cfg(test)]
    pub(super) async fn outcome(&self) -> Outcome {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let result = { lock(&self.state).outcome.clone() };
            if let Some(result) = result {
                return result;
            }
            changed.await;
        }
    }
}
