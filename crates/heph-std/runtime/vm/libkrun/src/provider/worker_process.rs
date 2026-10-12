//! Exact raw Child custody, independent of whether worker IPC is configured.
use super::common::{
    Arc, Child, ErrorSnapshot, Mutex, PROCESS_POLL_INTERVAL, VmError, error, info, sleep, watch,
};
use super::{
    helpers::{ProcessStatus, provider_error},
    worker_client::publish_process_poll,
};

pub(super) struct WorkerProcess {
    child: Mutex<Child>,
    pub(super) pid: Option<u32>,
    process_exit: watch::Sender<Option<ProcessStatus>>,
    process_error: watch::Sender<Option<ErrorSnapshot>>,
}

impl WorkerProcess {
    pub(super) fn retain(child: Child) -> Arc<Self> {
        let process = Arc::new(Self {
            pid: child.id(),
            child: Mutex::new(child),
            process_exit: watch::channel(None).0,
            process_error: watch::channel(None).0,
        });
        process.spawn_reaper();
        process
    }

    pub(super) fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        self.process_exit.subscribe()
    }

    pub(super) async fn kill(&self) -> Result<(), VmError> {
        let mut child = self.child.lock().await;
        if publish_process_poll(child.try_wait(), &self.process_exit, &self.process_error)?
            .is_none()
        {
            child
                .start_kill()
                .map_err(|error| provider_error("worker-kill", error))?;
        }
        drop(child);
        Ok(())
    }

    pub(super) async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        let mut exit = self.process_exit.subscribe();
        let mut errors = self.process_error.subscribe();
        let poll = {
            let mut child = self.child.lock().await;
            let result =
                publish_process_poll(child.try_wait(), &self.process_exit, &self.process_error);
            drop(child);
            result
        };
        if let Some(status) = poll? {
            return Ok(status);
        }
        loop {
            let status = exit.borrow_and_update().clone();
            if let Some(status) = status {
                return Ok(status);
            }
            let error = errors.borrow_and_update().clone();
            if let Some(error) = error {
                return Err(error.to_vm_error());
            }
            tokio::select! {
                result = exit.changed() => result.map_err(|error| provider_error("worker-exit-channel", error))?,
                result = errors.changed() => result.map_err(|error| provider_error("worker-reap-channel", error))?,
            }
        }
    }

    fn spawn_reaper(self: &Arc<Self>) {
        let process = Arc::clone(self);
        tokio::spawn(async move {
            let mut last_error = None;
            loop {
                let status = {
                    let mut child = process.child.lock().await;
                    publish_process_poll(
                        child.try_wait(),
                        &process.process_exit,
                        &process.process_error,
                    )
                };
                match status {
                    Ok(Some(status)) => {
                        info!(worker_pid = ?process.pid, code = ?status.code, signal = ?status.signal, "worker reaped");
                        break;
                    }
                    Ok(None) => {
                        last_error = None;
                        sleep(PROCESS_POLL_INTERVAL).await;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_ref() != Some(&message) {
                            error!(worker_pid = ?process.pid, %error, "failed to reap worker");
                            last_error = Some(message);
                        }
                        // Keep the exact Child through uncertainty, never a
                        // fabricated exit or a PID-based replacement process.
                        sleep(PROCESS_POLL_INTERVAL).await;
                    }
                }
            }
        });
    }
}
