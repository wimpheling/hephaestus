//! Original memory-only attempts. No host IO/native cleanup assertion.
use crate::FakeProvider;
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vm_trait::{VmError, VmInstance, VmProvider, VmProvisionAttempt, VmSpec};

struct State {
    spec: Option<VmSpec>,
    cleaned: bool,
    outcome: Option<Result<Arc<dyn VmInstance>, Arc<VmError>>>,
}
struct Original {
    provider: FakeProvider,
    closed: AtomicBool,
    state: tokio::sync::Mutex<State>,
}
struct Attempt(Arc<Original>);
pub fn prepare(provider: FakeProvider, spec: VmSpec) -> Arc<dyn VmProvisionAttempt> {
    Arc::new(Attempt(Arc::new(Original {
        provider,
        closed: AtomicBool::new(false),
        state: tokio::sync::Mutex::new(State {
            spec: Some(spec),
            cleaned: false,
            outcome: None,
        }),
    })))
}
#[async_trait]
impl VmProvisionAttempt for Attempt {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        let original = self.0.clone();
        let (reply, receive) = tokio::sync::oneshot::channel();
        drop(tokio::spawn(async move {
            let mut state = original.state.lock().await;
            let result = if original.closed.load(Ordering::SeqCst) {
                Err(VmError::Destroyed)
            } else if let Some(outcome) = &state.outcome {
                outcome.clone().map_err(retained)
            } else if let Some(spec) = state.spec.take() {
                // Fake::provision has no suspension/host IO. Every failure is
                // before insertion; success pins original instance before reply.
                let outcome = original.provider.provision(spec).await.map_err(Arc::new);
                state.outcome = Some(outcome.clone());
                outcome.map_err(retained)
            } else {
                Err(VmError::InvalidState(
                    "fake original attempt outcome unknown",
                ))
            };
            let result = if original.closed.load(Ordering::SeqCst) && result.is_ok() {
                Err(VmError::Destroyed)
            } else {
                result
            };
            drop(state);
            let _sent = reply.send(result);
        }));
        receive.await.map_err(channel_error)?
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        self.0.closed.store(true, Ordering::SeqCst);
        let original = self.0.clone();
        let (reply, receive) = tokio::sync::oneshot::channel();
        drop(tokio::spawn(async move {
            let mut state = original.state.lock().await;
            let result = if state.cleaned {
                Ok(())
            } else {
                match &state.outcome {
                    Some(Ok(instance)) => instance.destroy().await,
                    // Positive memory-only untouched/failed validation branch.
                    Some(Err(_)) => Ok(()),
                    None if state.spec.is_some() => Ok(()),
                    None => Err(VmError::InvalidState(
                        "fake original attempt outcome unknown",
                    )),
                }
            };
            if result.is_ok() {
                state.cleaned = true;
            }
            drop(state);
            let _sent = reply.send(result);
        }));
        receive.await.map_err(channel_error)?
    }
}
#[derive(Debug)]
struct OriginalError(Arc<VmError>);
impl std::fmt::Display for OriginalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl std::error::Error for OriginalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
fn retained(error: Arc<VmError>) -> VmError {
    VmError::Provider {
        provider: "fake".into(),
        code: "original-attempt".into(),
        source: Box::new(OriginalError(error)),
    }
}
fn channel_error(error: tokio::sync::oneshot::error::RecvError) -> VmError {
    VmError::Provider {
        provider: "fake".into(),
        code: "original-attempt-reply".into(),
        source: Box::new(error),
    }
}
