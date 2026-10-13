//! One original memory fixture attempt, not native or orphan absence proof.
#[cfg(test)]
#[path = "provision_attempt_tests.rs"]
mod tests;

use super::{ResultFixtureInstance, VmError, VmInstance, VmSpec};
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use vm_trait::VmProvisionAttempt;

struct State {
    started: bool,
    outcome: Option<Result<Arc<dyn VmInstance>, Arc<VmError>>>,
    cleaned: bool,
}
struct Attempt {
    spec: VmSpec,
    closed: AtomicBool,
    state: Mutex<State>,
}
pub(super) fn prepare(spec: VmSpec) -> Arc<dyn VmProvisionAttempt> {
    Arc::new(Attempt::new(spec))
}
impl Attempt {
    fn new(spec: VmSpec) -> Self {
        Self {
            spec,
            closed: AtomicBool::new(false),
            state: Mutex::new(State {
                started: false,
                outcome: None,
                cleaned: false,
            }),
        }
    }
}
#[async_trait]
impl VmProvisionAttempt for Attempt {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        let mut state = self.state.lock().await;
        // Original ResultFixtureInstance::new has ZERO awaits/host effects.
        // After this sole suspension, construction/cache complete in one poll.
        // This is fixture-specific, never a native cancellation guarantee.
        let outcome = if self.closed.load(Ordering::SeqCst) {
            Err(VmError::Destroyed)
        } else if let Some(outcome) = &state.outcome {
            outcome.clone().map_err(retained)
        } else if !state.started {
            state.started = true; // Missing result after unwind denies a new birth.
            let outcome = ResultFixtureInstance::new(self.spec.clone())
                .map(|instance| Arc::new(instance) as Arc<dyn VmInstance>)
                .map_err(Arc::new);
            state.outcome = Some(outcome.clone());
            outcome.map_err(retained)
        } else {
            Err(VmError::InvalidState("fixture attempt outcome unknown"))
        };
        let outcome = if self.closed.load(Ordering::SeqCst) && outcome.is_ok() {
            Err(VmError::Destroyed)
        } else {
            outcome
        };
        drop(state);
        outcome
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        self.closed.store(true, Ordering::SeqCst);
        let mut state = self.state.lock().await;
        // The unchanged original destroy is an immediate memory-fixture poll.
        // No suspension/effects beyond mutex acquisition are hidden here.
        let outcome = if state.cleaned {
            Ok(())
        } else {
            match &state.outcome {
                Some(Ok(instance)) => instance.destroy().await,
                // Every actual constructor error precedes instance/channel
                // allocation and all host IO. No name-based cleanup is needed.
                Some(Err(_)) => Ok(()),
                None if !state.started => Ok(()),
                None => Err(VmError::InvalidState("fixture attempt outcome unknown")),
            }
        };
        if outcome.is_ok() {
            state.cleaned = true;
        }
        drop(state);
        outcome
    }
}

#[derive(Debug)]
struct RetainedError(Arc<VmError>);
impl std::fmt::Display for RetainedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0.as_ref(), formatter)
    }
}
impl std::error::Error for RetainedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
fn retained(error: Arc<VmError>) -> VmError {
    VmError::Provider {
        provider: "local-result-fixture".into(),
        code: "original-attempt".into(),
        source: Box::new(RetainedError(error)),
    }
}
