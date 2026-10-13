//! Existing original Build cleanup; no OCI/Gateway or prior-process proof.
use super::{AppError, Body, State, joins::JoinSlot};
use std::sync::Arc;
use tokio::sync::oneshot;

pub(super) fn initiate(state: &mut State, retry: bool) -> Result<(), AppError> {
    if !state.closed {
        return Ok(());
    }
    let app = match &state.body {
        Body::Starting(app) | Body::App(app) | Body::Running { app, .. } => Arc::clone(app),
        Body::Empty => return Ok(()),
        #[cfg(test)]
        Body::TestOwner(_) => return Ok(()),
    };
    app.build_executor.close_admission();
    if let Some(previous) = state.build.last() {
        // Explicit same-owner retry may replace a known bounded observation error,
        // retaining every old handle/result. Raw panic/lost outcome remains held.
        if !retry || !matches!(previous.snapshot(), Some(Ok(Err(_)))) {
            drop(app);
            return Ok(());
        }
    }
    let (admit, admitted) = oneshot::channel();
    let task = tokio::spawn(async move {
        let original_app = app;
        admitted
            .await
            .map_err(|_| Arc::new(AppError::Task("original Build cleanup handoff lost".into())))?;
        let result = original_app
            .build_executor
            .close_and_cleanup(original_app.shutdown_timeout)
            .await
            .map_err(|cause| Arc::new(crate::managed_backend::custody::failure(cause)));
        drop(original_app);
        result
    });
    state.build.push(JoinSlot::new(task));
    admit
        .send(())
        .map_err(|()| AppError::Task("original Build cleanup task registration failed".into()))
}
