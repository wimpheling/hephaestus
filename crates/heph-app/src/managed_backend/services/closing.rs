//! Original App/Running remain node-owned through every closing await and unwind.
use super::{AppError, Body, Services};

pub(super) async fn run(services: &Services) -> Result<(), AppError> {
    services.request_close();
    let (startup, body) = {
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
        (state.startup.take(), state.body.clone())
    };
    let result = async {
        if let Some(startup) = startup {
            startup
                .await
                .map_err(crate::managed_backend::custody::failure)?;
        }
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
                services.tasks.join().await?;
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
            Body::Empty => services.tasks.join().await,
            #[cfg(test)]
            Body::TestOwner(_) => Err(AppError::Readiness(
                "controlled owner cannot qualify cleanup".into(),
            )),
        }
    }
    .await;
    // No original is removed on error. Even an unwound job leaves the actual
    // body plus closing marker retained; a marker cannot establish cleanup.
    let retired = {
        let mut state = services
            .state
            .lock()
            .map_err(|_| AppError::Task("managed service slot poisoned".into()))?;
        state.closing = false;
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
