//! Immediate transfer of actual handles; error/drop never aborts admitted work.
use super::joins::JoinSlot;
use crate::AppError;
use std::{
    error::Error,
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

type ServiceTask = Arc<JoinSlot<Result<(), String>>>;

#[derive(Default)]
pub(super) struct TaskCustody {
    handles: Mutex<Vec<ServiceTask>>,
}
impl TaskCustody {
    pub(super) fn retain(&self, handles: &mut Vec<JoinHandle<Result<(), String>>>) {
        // Poisoning preserves the actual handles instead of replacing the registry.
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(handles.drain(..).map(JoinSlot::new));
    }
    pub(super) fn healthy(&self) -> bool {
        self.handles.lock().is_ok_and(|handles| {
            !handles.is_empty() && handles.iter().all(|handle| handle.running())
        })
    }
    pub(super) fn settled(&self) -> Result<bool, AppError> {
        self.handles
            .lock()
            .map(|handles| handles.iter().all(|handle| handle.settled()))
            .map_err(|_| AppError::Task("managed service handle registry poisoned".into()))
    }
    pub(super) async fn join(&self) -> Result<(), AppError> {
        loop {
            let handles = self
                .handles
                .lock()
                .map_err(|_| AppError::Task("managed service handle registry poisoned".into()))?
                .clone();
            let mut failures = Vec::new();
            for handle in handles {
                match handle.join().await {
                    Ok(Ok(())) => {}
                    Ok(Err(cause)) => failures.push(AppError::Task(cause)),
                    Err(cause) => failures.push(cause),
                }
            }
            // SAME registry lock linearizes this snapshot against late transferred handles.
            let settled = self
                .handles
                .lock()
                .map_err(|_| AppError::Task("managed service handle registry poisoned".into()))?
                .iter()
                .all(|handle| handle.settled());
            if settled {
                return if failures.is_empty() {
                    Ok(())
                } else {
                    Err(crate::managed_backend::custody::failure(
                        ServiceJoinFailures(failures),
                    ))
                };
            }
        }
    }
}

pub struct OwnedTasks {
    plain: Vec<JoinHandle<Result<(), String>>>,
    custody: Option<Arc<TaskCustody>>,
}
impl OwnedTasks {
    pub fn new(services: Option<&super::Services>) -> Self {
        Self {
            plain: Vec::new(),
            custody: services.map(|services| Arc::clone(&services.tasks)),
        }
    }
    pub fn push(&mut self, task: JoinHandle<Result<(), String>>) {
        self.plain.push(task);
        self.capture();
    }
    pub fn capture(&mut self) {
        if let Some(custody) = &self.custody {
            custody.retain(&mut self.plain);
        }
    }
    pub const fn is_managed(&self) -> bool {
        self.custody.is_some()
    }
    pub fn into_plain(mut self) -> Vec<JoinHandle<Result<(), String>>> {
        self.capture();
        std::mem::take(&mut self.plain)
    }
}
impl Deref for OwnedTasks {
    type Target = Vec<JoinHandle<Result<(), String>>>;
    fn deref(&self) -> &Self::Target {
        &self.plain
    }
}
impl DerefMut for OwnedTasks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.plain
    }
}
impl Drop for OwnedTasks {
    fn drop(&mut self) {
        self.capture();
    }
}

#[derive(Debug)]
struct ServiceJoinFailures(Vec<AppError>);
impl fmt::Display for ServiceJoinFailures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "original service joins retained {} failures",
            self.0.len()
        )?;
        for cause in &self.0 {
            write!(f, "; {cause}")?;
        }
        Ok(())
    }
}
impl Error for ServiceJoinFailures {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.first().map(|cause| cause as &(dyn Error + 'static))
    }
}
