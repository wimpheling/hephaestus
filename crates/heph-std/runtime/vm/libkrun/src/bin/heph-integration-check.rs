//! Guest-side hardware integration probe for the libkrun backend.

use std::time::Duration;
use std::{
    io::{self, Write},
    net::ToSocketAddrs,
};

#[path = "heph-integration-check/broker.rs"]
mod broker;
#[path = "heph-integration-check/checks.rs"]
mod checks;
#[path = "heph-integration-check/handlers.rs"]
mod handlers;
#[path = "heph-integration-check/service_protocol.rs"]
mod service_protocol;
#[path = "heph-integration-check/service_server.rs"]
mod service_server;

const BROKERED_E2E_RULE_ID: &str = "00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_PLACEHOLDER: &str = "heph-placeholder:v1:00000000-0000-0000-0000-000000000002";
const BROKERED_E2E_CREDENTIAL_PATH: &str = "/run/hephaestus-secrets/.runtime-credential";
const SERVICE_DEFAULT_PORT: u16 = 8080;
const SERVICE_MAX_REQUESTS: usize = 128;
const SERVICE_MAX_CONNECTIONS: usize = 4;
const SERVICE_MAX_REQUEST_BYTES: usize = 16 * 1024;
const SERVICE_MAX_HEADER_COUNT: usize = 32;
const SERVICE_MAX_HEADER_LINE_BYTES: usize = 4 * 1024;
const SERVICE_MAX_BODY_BYTES: usize = 16 * 1024;
const SERVICE_MAX_DELAY_MS: u64 = 5_000;
const SERVICE_HOLD: Duration = Duration::from_secs(20);
const SERVICE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const SERVICE_CRASH_EXIT_CODE: i32 = 42;
const SERVICE_ISOLATION_CHECK_ENV: &str = "HEPH_SERVICE_ISOLATION_CHECK";
const RUNTIME_AUTHORITY_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";
const RUNTIME_AUTHORITY_FILE: &str = "/run/hephaestus-authority/session.json";
const RUNTIME_GIT_CREDENTIAL_HELPER: &str = "/usr/libexec/hephaestus/heph-git-credential";

fn main() {
    if let Err(error) = run() {
        let _write_result = writeln!(io::stderr(), "integration-check: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("--private-http-handler") => {
            return handlers::private_http_handler().map_err(Into::into);
        }
        Some("--private-http-brokered-header") => {
            return handlers::private_http_brokered_header_handler().map_err(Into::into);
        }
        Some("--private-http-brokered-mailbox") => {
            return handlers::private_http_brokered_mailbox_handler().map_err(Into::into);
        }
        Some("--serve-http") => return service_server::serve_http().map_err(Into::into),
        Some("--serve-service") => return service_server::serve_service().map_err(Into::into),
        Some("--runtime-git-http") => {
            let repository = std::env::args()
                .nth(2)
                .ok_or("runtime Git repository argument is missing")?;
            return handlers::runtime_git_http(&repository).map_err(Into::into);
        }
        Some("--expect-network-disabled") => return broker::expect_network_disabled(),
        Some("--expect-broker-only") => return broker::expect_broker_only(),
        Some("--brokered-https-e2e") => return broker::brokered_https_e2e(),
        Some("--expect-mailbox") => return handlers::expect_mailbox(),
        Some("--ignore-cancellation") => return broker::ignore_cancellation(),
        Some("--state-only") => {
            checks::verify_disk()?;
            println!("sqlite=ok");
            if let Ok(marker) = std::env::var("HEPH_RELEASE_MARKER") {
                println!("release_marker={marker}");
            }
            if let Ok(milliseconds) = std::env::var("HEPH_STATE_HOLD_MS") {
                std::thread::sleep(Duration::from_millis(milliseconds.parse()?));
            }
            return Ok(());
        }
        Some("--state-rollback") => {
            checks::verify_sqlite_rollback()?;
            println!("sqlite-rollback=ok");
            std::process::exit(23);
        }
        Some(argument) => {
            return Err(format!("unknown integration-check argument: {argument}").into());
        }
        None => {}
    }

    eprintln!("stderr=ok");
    checks::verify_disk()?;
    println!("sqlite=ok");
    checks::verify_mounts()?;
    println!("mounts=ok");
    if std::env::var("HEPH_EXPECT_SECRET_MOUNT").as_deref() == Ok("1") {
        checks::verify_secrets()?;
        println!("secrets=ok");
    }
    let resolved = ("example.com", 80)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS returned no addresses")?;
    println!("dns=ok");
    checks::verify_tcp(resolved)?;
    println!("tcp=ok");
    checks::verify_udp_dns()?;
    println!("udp=ok");
    Ok(())
}
