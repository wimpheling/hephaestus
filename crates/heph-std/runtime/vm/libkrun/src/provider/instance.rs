use super::common::{
    Arc, BoxedPrivateServiceConnection, LibkrunInstance, Lifecycle, Ordering, PRIVATE_HTTP_TIMEOUT,
    PROVIDER_NAME, PrivateServiceConnectionMessage, StopMode, VmError, VmEvent, VmExit, VmId,
    VmInstance, WorkerCommand, async_trait, broadcast, oneshot, timeout,
};

use super::{
    helpers::{provider_error, unavailable_error},
    http::private_http_request_message,
};

#[async_trait]
impl VmInstance for LibkrunInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        self.run_lifecycle(super::lifecycle_task::Request::Start)
            .await
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        self.run_lifecycle(super::lifecycle_task::Request::Stop(mode))
            .await
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        self.wait_for_terminal().await
    }

    async fn invoke_private_http(
        &self,
        request: vm_trait::PrivateHttpRequest,
    ) -> Result<vm_trait::PrivateHttpResponse, VmError> {
        if !self.private_http_enabled {
            return Err(VmError::Unsupported {
                feature: "private HTTP requires the declared http.v1 gateway handler contract"
                    .to_owned(),
                provider: PROVIDER_NAME.to_owned(),
            });
        }
        let request = private_http_request_message(request)?;
        let request_id = self
            .next_private_http_request
            .fetch_add(1, Ordering::Relaxed);
        if request_id == 0 {
            return Err(VmError::InvalidState(
                "private HTTP request identifier overflowed",
            ));
        }
        let (sender, receiver) = oneshot::channel();
        self.private_http_waiters
            .lock()
            .await
            .insert(request_id, sender);
        if let Err(error) = self
            .worker
            .request(WorkerCommand::InvokePrivateHttp {
                request_id,
                request,
            })
            .await
        {
            self.private_http_waiters.lock().await.remove(&request_id);
            return Err(error);
        }
        match timeout(PRIVATE_HTTP_TIMEOUT, receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => Err(provider_error("private-http-response", error)),
            Err(_) => {
                self.private_http_waiters.lock().await.remove(&request_id);
                Err(unavailable_error(
                    "private HTTP handler",
                    "response timed out",
                ))
            }
        }
    }

    // The offer must remain owned by the timeout future so cancellation drops
    // it and releases its pending broker slot.
    #[allow(clippy::significant_drop_tightening)]
    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        let timeout_duration =
            self.private_service_timeout
                .ok_or_else(|| VmError::Unsupported {
                    feature: "private HTTP service is not declared for this VM".to_owned(),
                    provider: PROVIDER_NAME.to_owned(),
                })?;
        {
            let state = self.state.lock().await;
            match &*state {
                Lifecycle::Running => {}
                Lifecycle::Destroying { .. } | Lifecycle::Destroyed => {
                    return Err(VmError::Destroyed);
                }
                Lifecycle::Exited => {
                    return Err(VmError::InvalidState("the VM has exited"));
                }
                Lifecycle::Provisioned | Lifecycle::Starting => {
                    return Err(VmError::InvalidState("the VM is not running"));
                }
                Lifecycle::Stopping => {
                    return Err(VmError::InvalidState("the VM is stopping"));
                }
                Lifecycle::StartFailed(_) => {
                    return Err(VmError::InvalidState("the VM failed to start"));
                }
            }
        }
        let dispatch = self.private_service_dispatch.clone().ok_or_else(|| {
            unavailable_error("private service broker", "dispatch is unavailable")
        })?;
        let permit = dispatch.try_acquire_owned().map_err(|error| {
            unavailable_error(
                "private service connection",
                match error {
                    tokio::sync::TryAcquireError::Closed => "dispatch is closed",
                    tokio::sync::TryAcquireError::NoPermits => "connection capacity is exhausted",
                },
            )
        })?;
        let broker = {
            let resources = self.resources.lock().await;
            resources
                .as_ref()
                .and_then(|owned| owned.service_broker.as_ref())
                .cloned()
        }
        .ok_or_else(|| unavailable_error("private service broker", "broker is unavailable"))?;
        let offer = broker
            .reserve()
            .map_err(|error| unavailable_error("private service connection", error.to_string()))?;
        let connection = PrivateServiceConnectionMessage {
            connection_id: offer.id(),
            challenge: offer.challenge().clone(),
        };
        let worker = Arc::clone(&self.worker);
        let control_task = tokio::spawn(async move {
            let _permit = permit;
            worker
                .request(WorkerCommand::OpenPrivateServiceConnection { connection })
                .await
        });
        let result = timeout(timeout_duration, async {
            control_task
                .await
                .map_err(|error| provider_error("worker-control-task", error))??;
            offer
                .connect()
                .await
                .map_err(|error| unavailable_error("private service connection", error.to_string()))
        })
        .await;
        result.map_or_else(
            |_| {
                Err(unavailable_error(
                    "private service connection",
                    "connection timed out",
                ))
            },
            |result| result.map(|connection| Box::new(connection) as BoxedPrivateServiceConnection),
        )
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        self.run_lifecycle(super::lifecycle_task::Request::Destroy)
            .await
    }
}
