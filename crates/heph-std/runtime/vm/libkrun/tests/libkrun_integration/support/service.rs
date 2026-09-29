use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use gateway_domain::ServiceProbePath;
use gateway_edge::{
    GatewayRequest, GatewayScheme, ServiceHttpPolicy, ServiceProbePolicy, TrustedRequestMetadata,
    exchange_private_service_http, probe_private_service_http,
};
use http::{HeaderMap, Method, StatusCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vm_trait::{VmEvent, VmInstance};

pub async fn poll_private_service(vm: &Arc<dyn VmInstance>, path: &str) -> (u16, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match private_service_request(vm, path).await {
                Ok(response) if response.0 == 200 => return response,
                Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("private service readiness polling timeout")
}

pub async fn poll_private_service_probe(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> gateway_edge::ServiceProbeSuccess {
    let path = ServiceProbePath::parse(path).expect("declared service probe path");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match probe_private_service_http(
                vm.as_ref(),
                &path,
                "service.internal",
                ServiceProbePolicy::new(Duration::from_secs(5)),
            )
            .await
            {
                Ok(response) => return response,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("private service probe readiness timeout")
}

pub async fn private_service_adapter_request(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> gateway_edge::GatewayResponse {
    let connection = vm
        .open_private_service_connection()
        .await
        .expect("open service connection for gateway-edge adapter");
    exchange_private_service_http(
        connection,
        GatewayRequest {
            method: Method::GET,
            path_and_query: path.to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Http,
                authority: String::from("service.internal"),
                client_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
                request_id: uuid::Uuid::new_v4(),
            },
        },
        ServiceHttpPolicy {
            max_request_body_bytes: 1,
            max_response_body_bytes: 64 * 1024,
            max_request_headers: 4,
            max_response_headers: 32,
            max_path_and_query_bytes: 512,
            max_wire_header_bytes: 8 * 1024,
            exchange_timeout: Duration::from_secs(5),
        },
    )
    .await
    .expect("gateway-edge private service exchange")
}

pub async fn private_service_request(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> io::Result<(u16, Vec<u8>)> {
    tokio::time::timeout(
        Duration::from_secs(5),
        private_service_request_inner(vm, path),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "private service request timeout"))?
}

pub async fn private_service_request_inner(
    vm: &Arc<dyn VmInstance>,
    path: &str,
) -> io::Result<(u16, Vec<u8>)> {
    const MAX_SERVICE_RESPONSE_BYTES: usize = 64 * 1024;
    let mut stream = vm
        .open_private_service_connection()
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: guest\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await?;
    let mut response = Vec::new();
    let mut limited = stream.take((MAX_SERVICE_RESPONSE_BYTES + 1) as u64);
    limited.read_to_end(&mut response).await?;
    if response.len() > MAX_SERVICE_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private service response exceeds test limit",
        ));
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "service response has no headers",
            )
        })?;
    let status = std::str::from_utf8(&response[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "service response is not UTF-8"))?
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "service response has no status")
        })?
        .parse::<u16>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "service response status is invalid",
            )
        })?;
    Ok((status, response[header_end + 4..].to_vec()))
}

pub fn parse_adapter_service_identity(
    response: &gateway_edge::GatewayResponse,
) -> serde_json::Value {
    assert_eq!(response.status, StatusCode::OK);
    serde_json::from_slice(&response.body).expect("service identity JSON")
}

pub async fn collect_logs_until_exit(
    events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut logs = String::new();
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Log { bytes, .. } => {
                    logs.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Exited(exit) => {
                    assert_eq!(exit.code, Some(0));
                    return logs;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout")
}

pub async fn collect_logs_until_any_exit(
    events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
) -> (String, vm_trait::VmExit) {
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut logs = String::new();
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Log { bytes, .. } => {
                    logs.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Exited(exit) => return (logs, exit),
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout")
}

pub async fn ready_http_host_port(events: &mut tokio::sync::broadcast::Receiver<VmEvent>) -> u16 {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut host_port = None;
        let mut guest_ready = false;
        loop {
            match events.recv().await.expect("HTTP startup event") {
                VmEvent::Started { ingress } => {
                    assert_eq!(ingress.len(), 1);
                    host_port = Some(ingress[0].host_port);
                }
                VmEvent::Log { bytes, .. }
                    if String::from_utf8_lossy(&bytes).contains("http=ready") =>
                {
                    guest_ready = true;
                }
                _ => {}
            }
            if let (Some(port), true) = (host_port, guest_ready) {
                return port;
            }
        }
    })
    .await
    .expect("HTTP startup event timeout")
}

pub async fn wait_for_log(events: &mut tokio::sync::broadcast::Receiver<VmEvent>, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let VmEvent::Log { bytes, .. } = events.recv().await.expect("guest log event") {
                if String::from_utf8_lossy(&bytes).contains(expected) {
                    return;
                }
            }
        }
    })
    .await
    .expect("guest log marker timeout");
}

pub async fn wait_for_service_isolation(events: &mut tokio::sync::broadcast::Receiver<VmEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut started = false;
        loop {
            match events.recv().await.expect("service guest event") {
                VmEvent::Started { ingress } => {
                    assert!(ingress.is_empty(), "private service received ingress");
                    started = true;
                }
                VmEvent::Log { bytes, .. }
                    if String::from_utf8_lossy(&bytes).contains("private-service-isolation=ok") =>
                {
                    assert!(started, "service isolation marker preceded VM start event");
                    return;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("service isolation marker timeout");
}
