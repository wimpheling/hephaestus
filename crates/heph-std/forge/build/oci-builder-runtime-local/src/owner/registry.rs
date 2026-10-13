use super::{Node, OciWorkerError, lock};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct State {
    closed: bool,
    nodes: BTreeMap<uuid::Uuid, Arc<Node>>,
}
#[derive(Default)]
pub(super) struct Registry(Mutex<State>);
impl Registry {
    pub(super) fn admit(
        &self,
        request: oci_builder_worker::IsolatedOciBuild,
        paths: [std::path::PathBuf; 3],
    ) -> Result<(Arc<Node>, bool, bool), OciWorkerError> {
        let mut state = lock(&self.0);
        if let Some(node) = state.nodes.get(&request.job_id) {
            // A conflicting retry is denied, but retains this actual original
            // so its caller cannot retire a possibly still-mounted checkout.
            return Ok((
                node.clone(),
                false,
                node.request == request && node.paths == paths,
            ));
        }
        if state.closed
            || request.job_id.is_nil()
            || !request.network_disabled
            || !request.ambient_credentials_disabled
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let node = Node::new(request, paths);
        state.nodes.insert(node.request.job_id, node.clone());
        let result = Ok((node, true, true));
        drop(state);
        result
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
    pub(super) fn get(&self, id: uuid::Uuid) -> Option<Arc<Node>> {
        lock(&self.0).nodes.get(&id).cloned()
    }
}
