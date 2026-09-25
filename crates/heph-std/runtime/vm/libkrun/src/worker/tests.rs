use super::guest::{validate_authority_sequence, validate_guest_message};
use crate::protocol::{GUEST_CONTROL_SOCKET_NAME, SUPERVISOR_SOCKET_NAME};
use crate::protocol::{
    GuestLogStream, GuestMessage, MAX_LOG_CHUNK_SIZE, MAX_METRIC_LABELS, MAX_METRIC_TEXT_SIZE,
    PROTOCOL_VERSION,
};
use std::{collections::BTreeMap, os::unix::net::UnixListener, path::Path};

#[cfg(target_os = "linux")]
#[test]
fn internal_socket_names_fit_long_oci_vm_paths() {
    // Keep the boundary fixture independent of a caller-provided TMPDIR:
    // a long temporary-root prefix would hide the realistic runtime path.
    let temporary = tempfile::tempdir_in("/tmp").expect("socket regression temporary root");
    // Match the 40-byte runtime prefix used by the GCE runner. The
    // `oci-builder-<uuid>` and `oci-verifier-<uuid>` IDs then reproduce
    // the path length that made guest-control.sock exceed AF_UNIX's limit.
    let prefix_target = 40_usize;
    let temp_len = temporary.path().as_os_str().len();
    let padding_len = prefix_target
        .checked_sub(temp_len + 18)
        .expect("temporary path must leave room for the realistic prefix");
    let runtime_prefix = temporary
        .path()
        .join("x".repeat(padding_len))
        .join(Path::new("h.EZsm0X/runtime"));
    assert_eq!(runtime_prefix.as_os_str().len(), prefix_target);

    for vm_id in [
        "oci-builder-13d83d0a-de07-4686-96d7-458e6a15682a",
        "oci-verifier-13d83d0a-de07-4686-96d7-458e6a15682a",
    ] {
        let runtime_dir = runtime_prefix.join(vm_id);
        std::fs::create_dir_all(&runtime_dir).expect("VM runtime directory");
        let legacy = runtime_dir.join("guest-control.sock");
        assert!(legacy.as_os_str().len() >= 108);
        assert!(UnixListener::bind(&legacy).is_err());

        let supervisor = UnixListener::bind(runtime_dir.join(SUPERVISOR_SOCKET_NAME))
            .expect("supervisor socket fits");
        let guest = UnixListener::bind(runtime_dir.join(GUEST_CONTROL_SOCKET_NAME))
            .expect("guest control socket fits");
        drop((supervisor, guest));
    }
}

#[test]
fn exit_with_code_and_signal_is_rejected() {
    assert!(
        validate_guest_message(&GuestMessage::Exited {
            code: Some(1),
            signal: Some(9),
        })
        .is_err()
    );
}

#[test]
fn duplicate_hello_is_rejected_after_handshake() {
    assert!(
        validate_guest_message(&GuestMessage::Hello {
            version: PROTOCOL_VERSION,
        })
        .is_err()
    );
}

#[test]
fn oversized_log_is_rejected() {
    assert!(
        validate_guest_message(&GuestMessage::Log {
            stream: GuestLogStream::Stdout,
            bytes: vec![0; MAX_LOG_CHUNK_SIZE + 1],
        })
        .is_err()
    );
}

#[test]
fn metric_bounds_are_enforced() {
    let oversized_name = GuestMessage::Metric {
        name: "x".repeat(MAX_METRIC_TEXT_SIZE + 1),
        value: 1.0,
        labels: BTreeMap::new(),
    };
    assert!(validate_guest_message(&oversized_name).is_err());

    let labels = (0..=MAX_METRIC_LABELS)
        .map(|index| (format!("key-{index}"), String::from("value")))
        .collect();
    assert!(
        validate_guest_message(&GuestMessage::Metric {
            name: String::from("metric"),
            value: 1.0,
            labels,
        })
        .is_err()
    );

    let invalid_label = GuestMessage::Metric {
        name: String::from("metric"),
        value: 1.0,
        labels: BTreeMap::from([(String::new(), String::from("value"))]),
    };
    assert!(validate_guest_message(&invalid_label).is_err());
}

#[test]
fn runtime_authority_acknowledgement_is_exact_and_precedes_ready() {
    let session_id = uuid::Uuid::new_v4();
    let expected = Some((session_id, 7));
    let mut acknowledged = false;
    assert!(
        validate_authority_sequence(expected, &mut acknowledged, &GuestMessage::Ready).is_err()
    );
    assert!(
        validate_authority_sequence(
            expected,
            &mut acknowledged,
            &GuestMessage::RuntimeAuthorityAcknowledged {
                session_id,
                generation: 8,
            },
        )
        .is_err()
    );
    validate_authority_sequence(
        expected,
        &mut acknowledged,
        &GuestMessage::RuntimeAuthorityAcknowledged {
            session_id,
            generation: 7,
        },
    )
    .expect("exact acknowledgement");
    validate_authority_sequence(expected, &mut acknowledged, &GuestMessage::Ready)
        .expect("ready after acknowledgement");
    assert!(
        validate_authority_sequence(
            expected,
            &mut acknowledged,
            &GuestMessage::RuntimeAuthorityAcknowledged {
                session_id,
                generation: 7,
            },
        )
        .is_err()
    );
}
