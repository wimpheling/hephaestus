//! Guest failure classification and bounded diagnostics.

use std::time::Duration;
use vm_trait::VmEvent;

pub fn operation_phase(operation: &'static str, stage: &'static str) -> &'static str {
    match (operation, stage) {
        ("builder", "startup") => "builder startup",
        ("builder", "execution") => "builder execution",
        ("verifier", "startup") => "verifier startup",
        ("verifier", "execution") => "verifier execution",
        _ => "operation execution",
    }
}

pub async fn guest_failure_phase(
    operation: &'static str,
    events: &mut tokio::sync::broadcast::Receiver<VmEvent>,
) -> &'static str {
    // Guest output can include repository-controlled Dockerfile data. Inspect
    // it only in memory for a fixed operational allowlist; never retain, log,
    // emit, or return the raw bytes.
    let mut tail = Vec::new();
    loop {
        let received = tokio::time::timeout(Duration::from_millis(250), events.recv()).await;
        match received {
            Ok(Ok(VmEvent::Log { bytes, .. })) => {
                retain_log_tail(&mut tail, &bytes);
            }
            Ok(Ok(VmEvent::Exited(_)) | Err(tokio::sync::broadcast::error::RecvError::Closed))
            | Err(_) => break,
            Ok(Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
        }
    }
    classify_guest_failure(operation, &tail)
}

pub fn retain_log_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    const LIMIT: usize = 16_384;
    if bytes.len() >= LIMIT {
        tail.clear();
        tail.extend_from_slice(&bytes[bytes.len() - LIMIT..]);
    } else {
        let discarded = (tail.len() + bytes.len()).saturating_sub(LIMIT);
        tail.drain(..discarded);
        tail.extend_from_slice(bytes);
    }
}

pub fn classify_guest_failure(operation: &'static str, output: &[u8]) -> &'static str {
    let lowered: Vec<_> = output.iter().map(u8::to_ascii_lowercase).collect();
    let contains = |needle: &[u8]| lowered.windows(needle.len()).any(|window| window == needle);
    if operation == "builder"
        && (contains(b"unshare") || contains(b"newuidmap") || contains(b"newgidmap"))
        && contains(b"operation not permitted")
    {
        "builder user-namespace setup"
    } else if operation == "builder" && contains(b"cannot set --network") {
        "builder network isolation contract"
    } else if operation == "builder" && contains(b"heph_oci_failure=base-import") {
        "builder approved-base import"
    } else if operation == "builder" && contains(b"heph_oci_failure=base-alias") {
        "builder approved-base alias"
    } else if operation == "builder" && contains(b"heph_oci_failure=dockerfile-build") {
        "builder Dockerfile build"
    } else if operation == "builder" && contains(b"heph_oci_failure=layout-export") {
        "builder layout export"
    } else if operation == "verifier" && contains(b"heph_oci_failure=output-cleanup") {
        "verifier output cleanup"
    } else if operation == "verifier" && contains(b"heph_oci_failure=output-prepare") {
        "verifier output preparation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootless-environment") {
        "verifier rootless environment"
    } else if operation == "verifier"
        && (contains(b"heph_oci_failure=cache-prepare") || contains(b"heph_oci_failure=cache-copy"))
    {
        "verifier offline scan cache"
    } else if operation == "verifier" && contains(b"heph_oci_failure=layout-validate") {
        "verifier OCI layout validation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=sbom") {
        "verifier SBOM generation"
    } else if operation == "verifier" && contains(b"heph_oci_failure=vulnerability-scan") {
        "verifier vulnerability scan"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootfs-export") {
        "verifier rootfs export"
    } else if operation == "verifier" && contains(b"heph_oci_failure=rootfs-handoff") {
        "verifier rootfs handoff"
    } else if operation == "verifier" && contains(b"heph_oci_failure=bundle-cleanup") {
        "verifier bundle cleanup"
    } else if operation == "verifier"
        && (contains(b"heph_oci_failure=manifest-read")
            || contains(b"heph_oci_failure=manifest-write"))
    {
        "verifier manifest output"
    } else {
        operation_phase(operation, "execution")
    }
}
