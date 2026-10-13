//! Original builtin-Git Child/IO custody, not arbitrary process subtree proof.
use super::{
    super::{
        BuildExecutionError,
        owner::{Node, lock},
    },
    git_io::Reader,
};
use std::{
    io,
    process::{Child, Command, ExitStatus, Output, Stdio},
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
    stdout: Option<Arc<Reader>>,
    stderr: Option<Arc<Reader>>,
    stopping: bool,
    errors: Vec<Arc<io::Error>>,
    diagnostics: Option<(Vec<u8>, Vec<u8>)>,
}
pub(in super::super) struct GitChild {
    state: Mutex<State>,
    captured: Condvar,
}
impl GitChild {
    pub(in super::super) fn spawn(
        command: &mut Command,
        node: &Arc<Node>,
    ) -> Result<Arc<Self>, BuildExecutionError> {
        node.check_open()?;
        let original = Arc::new(Self {
            state: Mutex::new(State {
                birth: Birth::Preparing,
                status: None,
                stdout: None,
                stderr: None,
                stopping: false,
                errors: Vec::new(),
                diagnostics: None,
            }),
            captured: Condvar::new(),
        });
        // Intent/slot retained before spawn. Spawn/capture uncertainty is NEVER
        // converted into absent resources or reconstructed from PID/name.
        node.git_child(original.clone())?;
        let result = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut state = lock(&original.state);
        match result {
            Ok(child) => state.birth = Birth::Original(child),
            Err(error) => {
                let error = Arc::new(error);
                state.errors.push(error.clone());
                state.birth = Birth::Failed(error.clone());
                original.captured.notify_all();
                return Err(BuildExecutionError::FilesystemSource(error));
            }
        }
        original.captured.notify_all();
        drop(state);
        Ok(original)
    }
    fn readers(&self) -> Result<(Arc<Reader>, Arc<Reader>), Arc<io::Error>> {
        let mut state = lock(&self.state);
        while matches!(state.birth, Birth::Preparing) {
            state = self
                .captured
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if let Birth::Failed(error) = &state.birth {
            return Err(error.clone());
        }
        if state.stdout.is_none() || state.stderr.is_none() {
            let Birth::Original(child) = &mut state.birth else {
                return Err(Arc::new(io::Error::other("original Git birth unknown")));
            };
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| Arc::new(io::Error::other("original Git stdout missing")))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| Arc::new(io::Error::other("original Git stderr missing")))?;
            // Both original descriptors retained in readers before polling IO.
            state.stdout = Some(Reader::new(stdout));
            state.stderr = Some(Reader::new(stderr));
        }
        let stdout = state
            .stdout
            .clone()
            .ok_or_else(|| Arc::new(io::Error::other("original Git stdout owner missing")))?;
        let stderr = state
            .stderr
            .clone()
            .ok_or_else(|| Arc::new(io::Error::other("original Git stderr owner missing")))?;
        drop(state);
        stdout.start();
        stderr.start();
        Ok((stdout, stderr))
    }
    fn wait(&self, stop: bool) -> Result<ExitStatus, Arc<io::Error>> {
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
                return Err(error.clone());
            }
            let stopping = state.stopping;
            let Birth::Original(child) = &mut state.birth else {
                return Err(Arc::new(io::Error::other("original Git child unknown")));
            };
            // Nonblocking observation avoids holding Child's mutex over a wait
            // that would prevent exact-child cancellation from another owner.
            match child.try_wait() {
                Ok(Some(_)) => {
                    let status = child.wait().map_err(Arc::new)?;
                    state.status = Some(status);
                    return Ok(status);
                }
                Ok(None) if stopping && !signaled => {
                    if let Err(error) = child.kill() {
                        let error = Arc::new(error);
                        state.errors.push(error.clone());
                        return Err(error);
                    }
                    signaled = true;
                }
                Ok(None) => {}
                Err(error) => {
                    let error = Arc::new(error);
                    state.errors.push(error.clone());
                    return Err(error);
                }
            }
            drop(state);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub(in super::super) fn output(&self) -> Result<Output, BuildExecutionError> {
        let (stdout, stderr) = self
            .readers()
            .map_err(BuildExecutionError::FilesystemSource)?;
        let status = self
            .wait(false)
            .map_err(BuildExecutionError::FilesystemSource)?;
        let stdout = stdout
            .take_bytes()
            .map_err(BuildExecutionError::FilesystemSource)?;
        let stderr = stderr
            .take_bytes()
            .map_err(BuildExecutionError::FilesystemSource)?;
        if !status.success() {
            lock(&self.state).diagnostics = Some((stdout.clone(), stderr.clone()));
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
    #[cfg(test)]
    pub(in super::super) fn observed_exit(&self) -> Option<ExitStatus> {
        lock(&self.state).status
    }
    #[cfg(test)]
    pub(in super::super) fn bounded_test_stop(&self, budget: Duration) -> bool {
        let deadline = std::time::Instant::now() + budget;
        loop {
            let mut state = lock(&self.state);
            if state.status.is_some() {
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
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub(in super::super) fn stop_and_join(&self) -> Result<(), BuildExecutionError> {
        // Kill request is not death: actual original wait comes before retiring
        // workspace, followed by joins of original pipe threads (including Err).
        // Command::spawn returned Err: the OS spawn call positively failed
        // before admission. Preserve its raw error, but no Child exists to join.
        // Preparing/capture panic remains unknown and must NOT take this branch.
        if matches!(lock(&self.state).birth, Birth::Failed(_)) {
            return Ok(());
        }
        self.wait(true)
            .map_err(BuildExecutionError::FilesystemSource)?;
        let (stdout, stderr) = self
            .readers()
            .map_err(BuildExecutionError::FilesystemSource)?;
        // Joined read errors are logical output failure, not live IO. Raw errors
        // remain in each retained Reader; missing/unjoined owners still deny.
        let stdout_result = stdout.join();
        let stderr_result = stderr.join();
        if !stdout.settled() || !stderr.settled() {
            return Err(BuildExecutionError::WorkerJoin);
        }
        drop(stdout_result);
        drop(stderr_result);
        Ok(())
    }
}
