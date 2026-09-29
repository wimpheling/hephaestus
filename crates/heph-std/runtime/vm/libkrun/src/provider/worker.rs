use super::{
    common::{
        VmError, WorkerBackend, WorkerClient, WorkerCommand, WorkerEvent, async_trait, broadcast,
        watch,
    },
    helpers::ProcessStatus,
};

#[async_trait]
impl WorkerBackend for WorkerClient {
    async fn request(&self, command: WorkerCommand) -> Result<(), VmError> {
        Self::request(self, command).await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<WorkerEvent> {
        Self::subscribe_events(self)
    }

    fn subscribe_process_exit(&self) -> watch::Receiver<Option<ProcessStatus>> {
        Self::subscribe_process_exit(self)
    }

    async fn kill(&self) -> Result<(), VmError> {
        Self::kill(self).await
    }

    async fn wait_process(&self) -> Result<ProcessStatus, VmError> {
        Self::wait_process(self).await
    }
}
