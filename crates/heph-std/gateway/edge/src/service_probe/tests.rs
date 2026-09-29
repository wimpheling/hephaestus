use super::*;
use async_trait::async_trait;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{broadcast, oneshot},
};
use vm_trait::{BoxedPrivateServiceConnection, StopMode, VmEvent, VmExit, VmId};

struct ProbeVm {
    id: VmId,
    connection: Mutex<Option<BoxedPrivateServiceConnection>>,
    open_delay: Duration,
    events: broadcast::Sender<VmEvent>,
}

impl ProbeVm {
    fn new(connection: BoxedPrivateServiceConnection, open_delay: Duration) -> Arc<Self> {
        let (events, _) = broadcast::channel(4);
        Arc::new(Self {
            id: VmId(String::from("probe-vm")),
            connection: Mutex::new(Some(connection)),
            open_delay,
            events,
        })
    }
}

#[async_trait]
impl VmInstance for ProbeVm {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        Ok(())
    }

    async fn stop(&self, _: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        Ok(VmExit {
            code: Some(0),
            signal: None,
        })
    }

    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        time::sleep(self.open_delay).await;
        self.connection
            .lock()
            .expect("probe connection lock")
            .take()
            .ok_or_else(|| VmError::Unavailable {
                resource: String::from("probe connection"),
                reason: String::from("connection already consumed"),
            })
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn path(value: &str) -> ServiceProbePath {
    ServiceProbePath::parse(value).expect("probe path")
}

fn pair(open_delay: Duration) -> (Arc<ProbeVm>, DuplexStream) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let vm = ProbeVm::new(Box::new(client), open_delay);
    (vm, server)
}

async fn read_request_headers(stream: &mut DuplexStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let count = stream.read(&mut buffer).await.expect("read probe request");
        assert_ne!(count, 0, "probe closed before request headers");
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return request;
        }
    }
}

#[tokio::test]
async fn uses_declared_path_and_trusted_host_without_caller_headers() {
    let (vm, mut server) = pair(Duration::ZERO);
    let server_task = tokio::spawn(async move {
        let request = read_request_headers(&mut server).await;
        let request = String::from_utf8(request).expect("HTTP request text");
        assert!(request.starts_with("GET /ready HTTP/1.1\r\n"));
        assert!(request.contains("host: service.internal\r\n"));
        assert!(request.contains("content-length: 0\r\n"));
        assert!(request.contains("connection: close\r\n"));
        assert!(!request.contains("authorization"));
        server
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write probe response");
    });
    vm.start().await.expect("control bootstrap");
    let result = probe_private_service_http(
        vm.as_ref(),
        &path("/ready"),
        "service.internal",
        ServiceProbePolicy::new(Duration::from_secs(1)),
    )
    .await
    .expect("successful readiness probe");
    assert_eq!(result.status, StatusCode::NO_CONTENT);
    server_task.await.expect("probe server task");
}

#[tokio::test]
async fn rejects_non_success_and_oversized_probe_responses() {
    for response in [
        &b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n"[..],
        &b"HTTP/1.1 200 OK\r\nContent-Length: 8193\r\n\r\n"[..],
    ] {
        let (vm, mut server) = pair(Duration::ZERO);
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            server
                .write_all(response)
                .await
                .expect("write probe response");
        });
        let result = probe_private_service_http(
            vm.as_ref(),
            &path("/health"),
            "service.internal",
            ServiceProbePolicy::new(Duration::from_secs(1)),
        )
        .await;
        match response[9] {
            b'5' => assert_eq!(
                result,
                Err(ServiceProbeError::NonSuccess(
                    StatusCode::SERVICE_UNAVAILABLE
                ))
            ),
            b'2' => assert_eq!(result, Err(ServiceProbeError::Contract)),
            status => panic!("unexpected fixture status {status}"),
        }
        server_task.await.expect("probe server task");
    }
}

#[tokio::test]
async fn deadline_covers_open_and_exchange_after_control_start() {
    let (vm, mut server) = pair(Duration::from_millis(15));
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        time::sleep(Duration::from_millis(25)).await;
        let _ = server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await;
    });
    vm.start().await.expect("control bootstrap alone");
    let result = probe_private_service_http(
        vm.as_ref(),
        &path("/ready"),
        "service.internal",
        ServiceProbePolicy::new(Duration::from_millis(30)),
    )
    .await;
    assert_eq!(result, Err(ServiceProbeError::TimedOut));
    server_task.await.expect("probe server task");
}

#[tokio::test]
async fn caller_cancellation_closes_probe_stream() {
    let (vm, mut server) = pair(Duration::ZERO);
    let (seen_tx, seen_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        seen_tx.send(()).expect("signal probe request");
        let mut byte = [0_u8; 1];
        let count = server.read(&mut byte).await.expect("observe probe close");
        assert_eq!(count, 0, "cancelled probe kept stream alive");
    });
    let probe = tokio::spawn({
        let vm = Arc::clone(&vm);
        async move {
            probe_private_service_http(
                vm.as_ref(),
                &path("/ready"),
                "service.internal",
                ServiceProbePolicy::new(Duration::from_secs(1)),
            )
            .await
        }
    });
    seen_rx.await.expect("probe request reached server");
    probe.abort();
    assert!(probe.await.expect_err("probe was cancelled").is_cancelled());
    server_task.await.expect("probe server task");
}
