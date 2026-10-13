//! Original App/Running remain node-owned through every closing await and unwind.
use super::{AppError, Body, Services};

pub(super) async fn run(services: &Services) -> Result<(), AppError> {
    services.request_close();
    let body = {
        let mut state = services
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        if state.closing {
            return Err(AppError::Readiness(
                "original managed closing remains unresolved".into(),
            ));
        }
        state.closing = true;
        let body = state.body.clone();
        drop(state);
        body
    };
    // This transient serialization marker is never a cleanup receipt.
    let _activity = ClosingActivity(services);
    let result = async {
        services.join_mutations().await?;
        // Startup may have installed Running while the owning join awaited.
        let current = services
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?
            .body
            .clone();
        match &current {
            Body::Running { app, running } => {
                if app.managed_backend.is_none() {
                    return Err(AppError::Readiness(
                        "original managed App lease missing".into(),
                    ));
                }
                qualify_ordinary_services(running)?;
                running.close_managed_services().await
            }
            Body::App(app) | Body::Starting(app) => {
                if app.managed_backend.is_none() {
                    Err(AppError::Readiness(
                        "retained managed App lost original lease".into(),
                    ))
                } else {
                    Err(ordinary_unavailable())
                }
            }
            Body::Empty => Err(ordinary_unavailable()),
            #[cfg(test)]
            Body::TestOwner(_) => Err(AppError::Readiness(
                "controlled owner cannot qualify cleanup".into(),
            )),
        }
    }
    .await;
    // No original is removed on error/unwind. Activity resets for same-owner
    // retry; closed intake and every actual body/slot/error remain retained.
    let retired = {
        let mut state = services
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        let retired = if result.is_ok() {
            Some(std::mem::replace(&mut state.body, Body::Empty))
        } else {
            if let Body::Starting(app) = &state.body {
                state.body = Body::App(std::sync::Arc::clone(app));
            }
            None
        };
        drop(state);
        retired
    };
    // Last lease Drop can request registry close. Never do that under slot lock.
    drop(retired);
    drop(body);
    result
}

fn qualify_ordinary_services(running: &crate::RunningHephaestus) -> Result<(), AppError> {
    if running.managed_backend.is_none() {
        return Err(AppError::Readiness(
            "retained managed Running bundle lost original lease".into(),
        ));
    }
    // Real ordinary close/physical cleanup must precede service joins.
    Err(ordinary_unavailable())
}
fn ordinary_unavailable() -> AppError {
    AppError::Readiness("ordinary Build/OCI/Gateway original-owner cleanup proof port is unavailable; original bundle/tasks/NATS/pools retained".into())
}

/// Observer cancellation/unwind releases serialization, never reopens admission.
struct ClosingActivity<'a>(&'a Services);
impl Drop for ClosingActivity<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closing = false;
        drop(state);
    }
}
