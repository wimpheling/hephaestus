use super::{BuildExecutionError, BuildRequestId, lock, models::Mode, node::Node};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
struct State {
    closed: bool,
    nodes: HashMap<BuildRequestId, Arc<Node>>,
    history: Vec<Arc<Node>>,
    errors: Vec<Arc<BuildExecutionError>>,
}
pub(in super::super) struct Registry(Mutex<State>);
impl Default for Registry {
    fn default() -> Self {
        Self(Mutex::new(State {
            closed: false,
            nodes: HashMap::new(),
            history: Vec::new(),
            errors: Vec::new(),
        }))
    }
}
pub(super) enum Admission {
    Follow(Arc<Node>),
    Start(Arc<Node>, Mode),
}
impl Registry {
    pub(super) fn register(
        &self,
        id: BuildRequestId,
        mode: Mode,
    ) -> Result<Admission, BuildExecutionError> {
        let mut registry = lock(&self.0);
        if let Some(node) = registry.nodes.get(&id).cloned() {
            let state = lock(&node.state);
            if state.outcome.is_none() {
                if state.mode == mode
                    || (state.mode == Mode::Finalize
                        && matches!(mode, Mode::Execute | Mode::Finalize))
                {
                    drop(state);
                    return Ok(Admission::Follow(node));
                }
                return Err(BuildExecutionError::AlreadyClaimed);
            }
            if state.finalization && matches!(mode, Mode::Execute | Mode::Finalize) {
                drop(state);
                if !lock(&node.execution_job)
                    .as_ref()
                    .is_some_and(|job| job.joined())
                    || !lock(&node.work_job)
                        .as_ref()
                        .is_some_and(|job| job.joined())
                    || lock(&node.cleanup_job)
                        .as_ref()
                        .is_some_and(|job| !job.joined())
                {
                    return Err(BuildExecutionError::AlreadyClaimed);
                }
                // Work-slot -> state order matches finish(): stale observers
                // cannot republish the old outcome during saved finalization.
                let mut work = lock(&node.work_job);
                let mut state = lock(&node.state);
                let previous = state
                    .outcome
                    .take()
                    .ok_or(BuildExecutionError::StoredState)?;
                state.history.push(previous);
                state.mode = Mode::Finalize;
                *work = None;
                drop(state);
                drop(work);
                return Ok(Admission::Start(node, Mode::Finalize));
            }
            if mode == Mode::Finalize {
                return Err(BuildExecutionError::FinalizationRequired);
            }
            drop(state);
            if !node.settled() {
                return Err(BuildExecutionError::VmCleanup);
            }
            if registry.closed {
                return Err(BuildExecutionError::Closed);
            }
            registry.history.push(node);
        }
        if mode == Mode::Finalize {
            return Err(BuildExecutionError::FinalizationRequired);
        }
        if registry.closed {
            return Err(BuildExecutionError::Closed);
        }
        let node = Node::new(id, mode);
        registry.nodes.insert(id, node.clone());
        drop(registry);
        Ok(Admission::Start(node, mode))
    }
    pub(super) fn root_error(&self, error: Arc<BuildExecutionError>) {
        lock(&self.0).errors.push(error);
    }
    pub(super) fn close(&self) {
        let mut state = lock(&self.0);
        state.closed = true;
        for node in state.nodes.values() {
            node.close();
        }
    }
    pub(super) fn nodes(&self) -> Vec<Arc<Node>> {
        lock(&self.0).nodes.values().cloned().collect()
    }
    pub(super) fn get(&self, id: BuildRequestId) -> Option<Arc<Node>> {
        lock(&self.0).nodes.get(&id).cloned()
    }
}
