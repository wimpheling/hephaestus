//! Concrete startup task owns the original App before its first awaited effect.
use super::{AppError, Body};
use crate::{
    HephaestusApp, RunningHephaestus,
    managed_backend::{
        custody::{self, ManagedBackendNode},
        supervisor::{ManagedHeldReason, registry::Registry},
    },
};
use std::sync::{Arc, Weak};
use tokio::sync::oneshot;

pub struct StartupTicket {
    result: oneshot::Receiver<Result<RunningHephaestus, AppError>>,
    registry: Weak<Registry>,
    delivered: bool,
}
impl StartupTicket {
    #[cfg(test)]
    pub(super) const fn test_ticket(
        result: oneshot::Receiver<Result<RunningHephaestus, AppError>>,
        registry: Weak<Registry>,
    ) -> Self {
        Self {
            result,
            registry,
            delivered: false,
        }
    }
    pub async fn wait(mut self) -> Result<RunningHephaestus, AppError> {
        let result = (&mut self.result).await.map_err(|_| {
            AppError::Task("managed startup result lost; original node remains held".into())
        })?;
        if result.is_ok() {
            self.delivered = true;
        }
        result
    }
}
impl Drop for StartupTicket {
    fn drop(&mut self) {
        if !self.delivered {
            if let Some(registry) = self.registry.upgrade() {
                registry.close();
            }
        }
    }
}

pub fn spawn(
    registry: Arc<Registry>,
    node: Arc<ManagedBackendNode>,
    app: HephaestusApp,
) -> Result<StartupTicket, AppError> {
    let app = Arc::new(app);
    registry.begin_services(&node, Arc::clone(&app))?;
    let weak = Arc::downgrade(&registry);
    let (result, receiver) = oneshot::channel();
    let (admit, admitted) = oneshot::channel();
    let retained = Arc::clone(&node);
    let task = tokio::spawn(async move {
        if admitted.await.is_err() {
            drop(node.services.put(Body::App(Arc::clone(&app))));
            registry.close();
            return;
        }
        if node.services.closed() {
            drop(node.services.put(Body::App(Arc::clone(&app))));
            let failure = registry.record(
                AppError::Readiness(
                    "managed startup closed before original service effects".into(),
                ),
                ManagedHeldReason::Services,
            );
            registry.close();
            drop(result.send(Err(custody::failure(failure))));
            return;
        }
        let outcome = crate::startup::managed_parts(&app, Arc::clone(&node.services)).await;
        match outcome {
            Ok(parts) => {
                let installed = crate::startup::running_borrowed(Arc::clone(&app), parts)
                    .and_then(|running| {
                        node.services.put(Body::Running {
                            app: Arc::clone(&app),
                            running: Arc::new(running),
                        })
                    })
                    .and_then(|()| registry.listeners_ready())
                    .and_then(|()| node.services.facade())
                    .and_then(|facade| {
                        facade
                            .managed_backend
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Readiness("managed facade lost original lease".into())
                            })?
                            .require_ready()?;
                        Ok(facade)
                    });
                match installed {
                    Ok(facade) => {
                        drop(result.send(Ok(facade)));
                    }
                    Err(cause) => {
                        let failure = registry.record(cause, ManagedHeldReason::Services);
                        registry.close();
                        drop(result.send(Err(custody::failure(failure))));
                    }
                }
            }
            Err(cause) => {
                drop(node.services.put(Body::App(Arc::clone(&app))));
                let failure = registry.record(cause, ManagedHeldReason::Services);
                registry.close();
                drop(result.send(Err(custody::failure(failure))));
            }
        }
    });
    if let Err(cause) = retained.services.attach(task) {
        drop(admit); // Actual late handle stays registered; its gated owner settles.
        return Err(cause);
    }
    admit
        .send(())
        .map_err(|()| AppError::Task("managed startup task handoff failed".into()))?;
    Ok(StartupTicket {
        result: receiver,
        registry: weak,
        delivered: false,
    })
}
