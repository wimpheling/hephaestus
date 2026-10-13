//! Existing exact runtime, raw Child and backend custody.
use crate::provider::{
    common::{Arc, Command, Duration, PathBuf, VmError, WorkerBackend, WorkerProcess, timeout},
    helpers::{provider_error, unavailable_error},
};
use std::fmt;

pub(in crate::provider) struct LaunchCustody {
    runtime: std::sync::Mutex<Option<crate::directory::ChildDirectory>>,
    process: std::sync::Mutex<Option<Arc<WorkerProcess>>>,
    backend: std::sync::Mutex<Option<Arc<dyn WorkerBackend>>>,
}

impl LaunchCustody {
    pub(super) fn new() -> Self {
        Self {
            runtime: std::sync::Mutex::new(None),
            process: std::sync::Mutex::new(None),
            backend: std::sync::Mutex::new(None),
        }
    }

    pub(in crate::provider) fn retain_runtime(
        &self,
        runtime: crate::directory::ChildDirectory,
    ) -> Result<(), VmError> {
        let mut slot = self.runtime.lock().map_err(|error| custody_error(&error))?;
        if slot.is_some() {
            return Err(VmError::InvalidState(
                "launch runtime identity already installed",
            ));
        }
        *slot = Some(runtime);
        drop(slot);
        Ok(())
    }

    pub(in crate::provider) fn runtime_file(&self) -> Result<Arc<std::fs::File>, VmError> {
        self.runtime
            .lock()
            .map_err(|error| custody_error(&error))?
            .as_ref()
            .ok_or(VmError::InvalidState("launch runtime descriptor missing"))?
            .file()
            .map_err(|error| provider_error("runtime-custody", error))
    }

    pub(in crate::provider) fn host_runtime_dir(&self) -> Result<PathBuf, VmError> {
        let runtime = self.runtime_file()?;
        Ok(crate::directory::fd_path(&runtime))
    }

    pub(in crate::provider) fn spawn(
        &self,
        command: &mut Command,
    ) -> Result<Arc<WorkerProcess>, VmError> {
        let mut slot = self.process.lock().map_err(|error| custody_error(&error))?;
        if slot.is_some()
            || self
                .backend
                .lock()
                .map_err(|error| custody_error(&error))?
                .is_some()
        {
            return Err(VmError::InvalidState("launch already owns a worker"));
        }
        let child = command
            .spawn()
            .map_err(|error| unavailable_error("worker binary", error.to_string()))?;
        let process = WorkerProcess::retain(child);
        // No await separates spawn from installation into the caller's owner.
        *slot = Some(Arc::clone(&process));
        drop(slot);
        Ok(process)
    }

    pub(in crate::provider) fn record_backend(
        &self,
        backend: Arc<dyn WorkerBackend>,
    ) -> Result<(), VmError> {
        let mut slot = self.backend.lock().map_err(|error| custody_error(&error))?;
        if let Some(current) = slot.as_ref() {
            if !Arc::ptr_eq(current, &backend) {
                return Err(VmError::InvalidState("launch backend identity changed"));
            }
        } else {
            *slot = Some(backend);
        }
        drop(slot);
        Ok(())
    }

    pub(in crate::provider) fn process(&self) -> Result<Option<Arc<WorkerProcess>>, VmError> {
        Ok(self
            .process
            .lock()
            .map_err(|error| custody_error(&error))?
            .clone())
    }

    pub(in crate::provider) async fn terminate(&self, grace: Duration) -> Result<(), VmError> {
        let backend = self
            .backend
            .lock()
            .map_err(|error| custody_error(&error))?
            .clone();
        if let Some(backend) = backend {
            backend.kill().await?;
            timeout(grace, backend.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
            return Ok(());
        }
        let process = self.process()?;
        if let Some(process) = process {
            process.kill().await?;
            timeout(grace, process.wait_process())
                .await
                .map_err(|_| unavailable_error("worker cleanup", "worker reap timed out"))??;
        }
        Ok(())
    }
}

fn custody_error(error: &impl fmt::Display) -> VmError {
    provider_error("launch-custody", std::io::Error::other(error.to_string()))
}
