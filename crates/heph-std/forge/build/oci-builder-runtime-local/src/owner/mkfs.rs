//! Exact formatter Child; parent death is NOT helper/subtree absence.
use super::{OciWorkerError, lock};
use std::{
    io,
    process::{Child, Command, ExitStatus},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
enum Birth {
    Preparing,
    Failed(Arc<io::Error>),
    Original(Child),
}
struct State {
    birth: Birth,
    status: Option<ExitStatus>,
    stopping: bool,
    errors: Vec<Arc<io::Error>>,
}
pub(super) struct Original {
    state: Mutex<State>,
    captured: Condvar,
    // Immutable actual trusted program/argv are private comparison evidence.
    _program: std::path::PathBuf,
    _arguments: Vec<std::ffi::OsString>,
}
impl Original {
    pub(super) fn new(command: &Command) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                birth: Birth::Preparing,
                status: None,
                stopping: false,
                errors: Vec::new(),
            }),
            captured: Condvar::new(),
            _program: command.get_program().into(),
            _arguments: command.get_args().map(std::ffi::OsStr::to_owned).collect(),
        })
    }
    pub(super) fn run(&self, command: &mut Command) -> Result<ExitStatus, OciWorkerError> {
        let result = command.spawn();
        let mut state = lock(&self.state);
        match result {
            Ok(child) => state.birth = Birth::Original(child),
            Err(error) => {
                let error = Arc::new(error);
                state.errors.push(error.clone());
                state.birth = Birth::Failed(error);
                self.captured.notify_all();
                return Err(OciWorkerError::Process(io::Error::other(
                    "original formatter spawn failed",
                )));
            }
        }
        self.captured.notify_all();
        drop(state);
        self.wait(false)
    }
    fn wait(&self, stop: bool) -> Result<ExitStatus, OciWorkerError> {
        let mut signaled = false;
        loop {
            let mut state = lock(&self.state);
            if stop {
                state.stopping = true;
            }
            if let Some(status) = state.status {
                return Ok(status);
            }
            while matches!(state.birth, Birth::Preparing) {
                state = self
                    .captured
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if let Birth::Failed(error) = &state.birth {
                return Err(OciWorkerError::Process(io::Error::new(
                    error.kind(),
                    "original formatter spawn failed",
                )));
            }
            let stopping = state.stopping;
            let Birth::Original(child) = &mut state.birth else {
                return Err(OciWorkerError::InvalidConfiguration);
            };
            match child.try_wait() {
                Ok(Some(_)) => match child.wait() {
                    Ok(status) => {
                        state.status = Some(status);
                        return Ok(status);
                    }
                    Err(error) => {
                        state.errors.push(Arc::new(error));
                        return Err(OciWorkerError::Process(io::Error::other(
                            "original formatter wait failed",
                        )));
                    }
                },
                Ok(None) if stopping && !signaled => {
                    if let Err(error) = child.kill() {
                        state.errors.push(Arc::new(error));
                        return Err(OciWorkerError::Process(io::Error::other(
                            "original formatter kill failed",
                        )));
                    }
                    signaled = true;
                }
                Ok(None) => {}
                Err(error) => {
                    state.errors.push(Arc::new(error));
                    return Err(OciWorkerError::Process(io::Error::other(
                        "original formatter observation failed",
                    )));
                }
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub(super) fn stop_and_join(&self) -> Result<(), OciWorkerError> {
        if matches!(lock(&self.state).birth, Birth::Failed(_)) {
            return Ok(());
        }
        self.wait(true).map(|_| ())
    }
    pub(super) fn parent_settled(&self) -> bool {
        let state = lock(&self.state);
        state.status.is_some() || matches!(state.birth, Birth::Failed(_))
    }
    pub(super) fn helper_unqualified(&self) -> bool {
        !matches!(lock(&self.state).birth, Birth::Failed(_))
    }
    #[cfg(test)]
    pub(super) fn status(&self) -> Option<ExitStatus> {
        lock(&self.state).status
    }
    #[cfg(test)]
    pub(super) fn bounded_stop(&self, budget: Duration) -> bool {
        let end = std::time::Instant::now() + budget;
        loop {
            let mut state = lock(&self.state);
            if state.status.is_some() || matches!(state.birth, Birth::Failed(_)) {
                return true;
            }
            let Birth::Original(child) = &mut state.birth else {
                return false;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.status = Some(status);
                    return true;
                }
                Ok(None) => {
                    if child.kill().is_err() {
                        return false;
                    }
                }
                Err(_) => return false,
            }
            drop(state);
            if std::time::Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
