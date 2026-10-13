//! One deterministic fixture attempt; no native process/orphan absence proof.
#[cfg(test)]
#[path = "attempt/tests.rs"]
mod tests;

use super::OutputProvider;
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use vm_trait::{VmError, VmInstance, VmProvider, VmProvisionAttempt, VmSpec};

struct State {
    started: bool,
    outcome: Option<Result<Arc<dyn VmInstance>, Arc<VmError>>>,
    cleaned: bool,
}
struct Original {
    provider: OutputProvider,
    spec: VmSpec,
    closed: AtomicBool,
    state: Mutex<State>,
}
struct Attempt(Arc<Original>);
pub(super) fn prepare(provider: OutputProvider, spec: VmSpec) -> Arc<dyn VmProvisionAttempt> {
    Arc::new(Attempt(Arc::new(Original {
        provider,
        spec,
        closed: AtomicBool::new(false),
        state: Mutex::new(State {
            started: false,
            outcome: None,
            cleaned: false,
        }),
    })))
}
#[async_trait]
impl VmProvisionAttempt for Attempt {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        let mut state = self.0.state.lock().await;
        // This fixture's original provision has ZERO internal awaits or process
        // allocation: after acquiring this mutex its poll completes immediately.
        // Cancellation before acquisition performs no effects. This premise is
        // specific to OutputProvider, not a native provisioning guarantee.
        let outcome = if self.0.closed.load(Ordering::SeqCst) {
            Err(VmError::Destroyed)
        } else if let Some(outcome) = &state.outcome {
            outcome.clone().map_err(retained)
        } else if !state.started {
            state.started = true; // An unwind cannot retry a replacement birth.
            let outcome = self
                .0
                .provider
                .provision(self.0.spec.clone())
                .await
                .map_err(Arc::new);
            state.outcome = Some(outcome.clone());
            outcome.map_err(retained)
        } else {
            Err(VmError::InvalidState("fixture attempt outcome unknown"))
        };
        let outcome = if self.0.closed.load(Ordering::SeqCst) && outcome.is_ok() {
            Err(VmError::Destroyed)
        } else {
            outcome
        };
        drop(state);
        outcome
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        self.0.closed.store(true, Ordering::SeqCst);
        let mut state = self.0.state.lock().await;
        // Original OutputInstance::destroy is also an immediate memory fixture
        // poll. There is no suspension after this operation acquires its mutex.
        let outcome = if state.cleaned {
            Ok(())
        } else {
            match &state.outcome {
                Some(Ok(instance)) => instance.destroy().await,
                // The unchanged producer's sole returned failure precedes
                // instance/output allocation; absent outcome after start is Held.
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
        provider: "isolated-build-test".into(),
        code: "original-attempt".into(),
        source: Box::new(RetainedError(error)),
    }
}
