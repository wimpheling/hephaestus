use brokered_egress_client::{BrokeredHttpsClient, WireBrokerRequest, WireBrokerStatus};
use runtime_types::RunId;
use serde::Deserialize;
use std::{
    fs, io,
    net::{SocketAddr, TcpStream},
    os::fd::FromRawFd,
    time::Duration,
};
use vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES;

use crate::{BROKERED_E2E_CREDENTIAL_PATH, BROKERED_E2E_PLACEHOLDER, BROKERED_E2E_RULE_ID};

pub fn expect_network_disabled() -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = "1.1.1.1:80".parse()?;
    if TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok() {
        return Err("disabled network unexpectedly allowed outbound TCP".into());
    }
    println!("network-disabled=ok");
    Ok(())
}

/// Proves the broker-only VM contract retains its dedicated vsock endpoint
/// while denying ordinary TCP egress.
pub fn expect_broker_only() -> Result<(), Box<dyn std::error::Error>> {
    expect_network_disabled()?;
    let mut authority = read_runtime_authority()?;
    let request_body = serde_json::json!({
        "rule_id": "00000000-0000-0000-0000-000000000002",
        "method": "get",
        "path_and_query": "/v1/probe",
        "headers": [],
        "body": []
    });
    let mut request = WireBrokerRequest {
        credential: std::mem::take(&mut authority.credential),
        run_id: RunId::from_uuid(authority.session_id),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&request_body)?,
    };
    let response = BrokeredHttpsClient::new(vsock_broker_stream()?).call(&request)?;
    request.credential.fill(0);
    if response.status != WireBrokerStatus::Succeeded || response.body != b"ok" {
        return Err("unexpected broker response".into());
    }
    println!("broker-vsock=ok");
    Ok(())
}

/// Exercises the real secret-runtime credential issued into the protected
/// brokered mount by the daemon's `PostgreSQL` resolver. The provider secret is
/// represented only by the stable placeholder in this released guest.
pub fn brokered_https_e2e() -> Result<(), Box<dyn std::error::Error>> {
    expect_network_disabled()?;
    let authority = read_runtime_authority()?;
    let mut credential = fs::read(BROKERED_E2E_CREDENTIAL_PATH)?;
    if credential.len() != RUNTIME_AUTHORITY_CREDENTIAL_BYTES {
        return Err("brokered runtime credential has an invalid length".into());
    }
    let request_body = serde_json::json!({
        "rule_id": BROKERED_E2E_RULE_ID,
        "method": "get",
        "path_and_query": "/v1/probe",
        "headers": [{
            "name": "authorization",
            "value": format!("Bearer {BROKERED_E2E_PLACEHOLDER}")
        }],
        "body": []
    });
    let mut request = WireBrokerRequest {
        credential: std::mem::take(&mut credential),
        // The daemon deliberately derives the generic runtime session ID from
        // the run ID, so this guest-visible non-secret identifier safely
        // names the exact secret-runtime lease without exposing its bearer.
        run_id: RunId::from_uuid(authority.session_id),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&request_body)?,
    };
    let response = BrokeredHttpsClient::new(vsock_broker_stream()?).call(&request)?;
    request.credential.fill(0);
    if response.status != WireBrokerStatus::Succeeded || response.body != b"brokered-e2e-ok" {
        return Err("unexpected brokered HTTPS response".into());
    }
    println!("brokered-https-e2e=ok");
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestRuntimeAuthority {
    session_id: uuid::Uuid,
    generation: u64,
    credential_hex: String,
}

struct RuntimeAuthorityCredential {
    session_id: uuid::Uuid,
    credential: Vec<u8>,
}

fn read_runtime_authority() -> Result<RuntimeAuthorityCredential, Box<dyn std::error::Error>> {
    let authority_path = std::env::var(vm_libkrun::protocol::RUNTIME_AUTHORITY_PATH_ENV)
        .unwrap_or_else(|_| String::from(vm_libkrun::protocol::GUEST_RUNTIME_AUTHORITY_PATH));
    let authority: GuestRuntimeAuthority = serde_json::from_slice(&fs::read(authority_path)?)?;
    if authority.generation == 0 {
        return Err("runtime authority generation is invalid".into());
    }
    if authority.credential_hex.len() != 64 {
        return Err("runtime authority credential has an invalid length".into());
    }
    let mut credential = Vec::with_capacity(32);
    for offset in (0..authority.credential_hex.len()).step_by(2) {
        credential.push(u8::from_str_radix(
            &authority.credential_hex[offset..offset + 2],
            16,
        )?);
    }
    Ok(RuntimeAuthorityCredential {
        session_id: authority.session_id,
        credential,
    })
}

#[allow(unsafe_code)] // AF_VSOCK is exposed only through libc's raw socket ABI.
fn vsock_broker_stream() -> io::Result<fs::File> {
    #[repr(C)]
    struct SockAddrVm {
        family: libc::sa_family_t,
        reserved: u16,
        port: u32,
        cid: u32,
        zero: [u8; 4],
    }

    // SAFETY: this guest fixture creates one AF_VSOCK stream and immediately
    // transfers its owned file descriptor to File after a checked connect.
    let descriptor = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    let family = libc::sa_family_t::try_from(libc::AF_VSOCK)
        .map_err(|_| io::Error::other("AF_VSOCK address family is invalid"))?;
    let address = SockAddrVm {
        family,
        reserved: 0,
        port: vm_libkrun::protocol::SECRET_BROKER_VSOCK_PORT,
        cid: 2, // VMADDR_CID_HOST is the stable host CID for libkrun guests.
        zero: [0; 4],
    };
    let length = libc::socklen_t::try_from(std::mem::size_of::<SockAddrVm>())
        .map_err(|_| io::Error::other("vsock address is oversized"))?;
    // SAFETY: address is a fully initialized repr(C) sockaddr_vm and its
    // pointer remains valid for the duration of this synchronous syscall.
    let connected = unsafe {
        libc::connect(
            descriptor,
            std::ptr::from_ref(&address).cast::<libc::sockaddr>(),
            length,
        )
    };
    if connected != 0 {
        // SAFETY: descriptor is still exclusively owned after a failed connect.
        let _closed = unsafe { libc::close(descriptor) };
        return Err(io::Error::last_os_error());
    }
    // SAFETY: connect succeeded and ownership transfers exactly once into File.
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[allow(unsafe_code)]
pub fn ignore_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: installing SIG_IGN for SIGTERM changes only this isolated guest
    // test process and does not dereference memory.
    let previous = unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    if previous == libc::SIG_ERR {
        return Err(io::Error::last_os_error().into());
    }
    println!("ignore-cancellation=ready");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
