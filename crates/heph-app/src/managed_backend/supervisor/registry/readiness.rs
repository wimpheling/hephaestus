//! Same registry lock linearizes listener publication and permanent close.
use super::{AppError, Arc, Held, ManagedBackendNode, ManagedServiceFailure, Phase, Registry};
impl Registry {
    pub fn begin_services(
        self: &Arc<Self>,
        node: &Arc<ManagedBackendNode>,
        app: Arc<crate::HephaestusApp>,
    ) -> Result<(), AppError> {
        let mut state = self.state();
        if state.poisoned
            || self.control.is_closing()
            || state.status.held_reason.is_some()
            || state.status.phase != Phase::Recovered
            || !state
                .node
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, node))
        {
            return Err(AppError::Readiness(
                "managed startup admission is stale, held or closing".into(),
            ));
        }
        node.services.begin(Arc::downgrade(self), app)?;
        state.refresh_activity();
        self.publish(&state);
        drop(state);
        Ok(())
    }
    pub fn is_closing(&self) -> bool {
        self.control.is_closing()
    }
    pub fn close(&self) {
        let mut state = self.state();
        if let Some(node) = &state.node {
            if let Err(cause) = crate::managed_backend::custody::close_intake(node) {
                state.failures.push(ManagedServiceFailure {
                    cause: Arc::new(cause),
                    reason: Held::Services,
                });
                state.status.held_reason = Some(Held::Services);
            }
        }
        self.control.close();
        if state.status.phase != Phase::Stopped {
            state.status.phase = Phase::Closing;
        }
        self.publish(&state);
        drop(state);
    }
    pub fn node(&self) -> Result<Arc<ManagedBackendNode>, AppError> {
        self.state()
            .node
            .clone()
            .ok_or_else(|| AppError::Readiness("managed original node is unavailable".into()))
    }
    pub fn require_recovered(&self) -> Result<(), AppError> {
        let state = self.state();
        if state.poisoned
            || self.control.is_closing()
            || state.status.held_reason.is_some()
            || !matches!(state.status.phase, Phase::Recovered | Phase::Ready)
            || state.node.is_none()
        {
            return Err(AppError::Readiness(
                "managed original recovery is pending, held or closing".into(),
            ));
        }
        drop(state);
        Ok(())
    }
    pub fn require_ready(&self) -> Result<(), AppError> {
        self.require_recovered()?;
        let state = self.state();
        if self.control.is_closing()
            || state.status.phase != Phase::Ready
            || !state
                .node
                .as_ref()
                .is_some_and(|node| node.services.healthy())
        {
            return Err(AppError::Readiness(
                "managed listeners are not positively Ready".into(),
            ));
        }
        drop(state);
        Ok(())
    }
    pub fn listeners_ready(&self) -> Result<(), AppError> {
        // close() and this publication share this exact lock; no closing winner
        // can be overwritten by a late listener result.
        let mut state = self.state();
        if state.poisoned
            || self.control.is_closing()
            || state.status.held_reason.is_some()
            || state.status.phase != Phase::Recovered
            || !state
                .node
                .as_ref()
                .is_some_and(|node| node.services.healthy())
        {
            return Err(AppError::Readiness(
                "managed listener publication is stale, held or closing".into(),
            ));
        }
        state.status.phase = Phase::Ready;
        state.refresh_activity();
        self.publish(&state);
        drop(state);
        Ok(())
    }
    pub fn check_services(&self) {
        let lost = {
            let state = self.state();
            state.status.phase == Phase::Ready
                && !state
                    .node
                    .as_ref()
                    .is_some_and(|node| node.services.healthy())
        };
        if lost {
            self.record(
                AppError::Readiness("an original managed service exited after Ready".into()),
                Held::Services,
            );
            self.close();
        }
    }
    pub fn stopped(&self) {
        let mut state = self.state();
        state.cleanup.finish();
        state.status.phase = Phase::Stopped;
        state.status.in_flight = false;
        state.status.held_reason = None;
        self.publish(&state);
        drop(state);
    }
}
