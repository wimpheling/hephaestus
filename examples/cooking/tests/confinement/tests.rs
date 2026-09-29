use super::{
    CREDENTIAL_PATTERNS, VmLogScan, assert_nats_has_no_credentials,
    assert_state_snapshot_has_no_credentials,
};
use futures_util::FutureExt as _;

#[test]
fn native_vm_logs_reject_split_encodings_without_joining_streams() {
    let run = uuid::Uuid::new_v4();
    for pattern in CREDENTIAL_PATTERNS.iter() {
        let midpoint = pattern.len() / 2;
        let first = serde_json::json!(&pattern[..midpoint]);
        let second = serde_json::json!(&pattern[midpoint..]);
        let detected = std::panic::catch_unwind(|| {
            let mut scan = VmLogScan::default();
            scan.inspect(run, "Stdout".to_owned(), &first);
            scan.inspect(run, "Stdout".to_owned(), &second);
        });
        assert!(detected.is_err(), "split native log encoding escaped scan");
        let mut scan = VmLogScan::default();
        scan.inspect(run, "Stderr".to_owned(), &first);
        scan.inspect(run, "Stdout".to_owned(), &second);
        let mut scan = VmLogScan::default();
        scan.inspect(run, "Stdout".to_owned(), &first);
        scan.inspect(uuid::Uuid::new_v4(), "Stdout".to_owned(), &second);
    }
}

#[test]
fn stored_build_logs_reject_split_text_encodings() {
    for pattern in CREDENTIAL_PATTERNS.iter() {
        let run = uuid::Uuid::new_v4();
        assert!(
            std::panic::catch_unwind(|| {
                let mut scan = VmLogScan::default();
                for chunk in pattern.chunks(3) {
                    scan.inspect_bytes(run, "stdout".to_owned(), chunk);
                }
            })
            .is_err(),
            "split build text encoding escaped scan"
        );
    }
}

#[test]
fn native_vm_logs_reject_malformed_byte_arrays() {
    for payload in [
        serde_json::json!(null),
        serde_json::json!([256]),
        serde_json::json!([-1]),
        serde_json::json!([1.5]),
        serde_json::json!(["65"]),
    ] {
        assert!(
            std::panic::catch_unwind(|| {
                VmLogScan::default().inspect(uuid::Uuid::new_v4(), "Stdout".to_owned(), &payload);
            })
            .is_err()
        );
    }
}

#[tokio::test]
#[ignore = "requires a dedicated disposable NATS server via HEPHAESTUS_CONFINEMENT_NATS_TEST_URL"]
async fn retained_nats_scan_rejects_payload_header_and_subject_credentials() {
    let url =
        std::env::var("HEPHAESTUS_CONFINEMENT_NATS_TEST_URL").expect("dedicated NATS fixture URL");
    let context = async_nats::jetstream::new(async_nats::connect(&url).await.unwrap());
    for (name, subject) in [
        ("HEPH_RUN_COMMANDS", "commands.>"),
        ("HEPHAESTUS_PRODUCT_EVENTS", "events.>"),
    ] {
        context
            .create_stream(async_nats::jetstream::stream::Config {
                name: name.to_owned(),
                subjects: vec![subject.to_owned()],
                ..Default::default()
            })
            .await
            .unwrap();
    }
    context
        .publish("events.safe", "safe".into())
        .await
        .unwrap()
        .await
        .unwrap();
    assert_nats_has_no_credentials(&url).await;
    let stream = context
        .get_stream("HEPHAESTUS_PRODUCT_EVENTS")
        .await
        .unwrap();
    for pattern in CREDENTIAL_PATTERNS.iter() {
        let sentinel = std::str::from_utf8(pattern).expect("ASCII fixture encoding");
        for surface in ["payload", "header", "subject"] {
            let mut headers = async_nats::HeaderMap::new();
            headers.insert(
                "Fixture",
                if surface == "header" {
                    sentinel
                } else {
                    "safe"
                },
            );
            let subject = if surface == "subject" {
                format!("events.{sentinel}")
            } else {
                String::from("events.safe")
            };
            let payload = if surface == "payload" {
                sentinel
            } else {
                "safe"
            };
            let ack = context
                .publish_with_headers(subject, headers, payload.to_owned().into())
                .await
                .unwrap()
                .await
                .unwrap();
            assert!(
                std::panic::AssertUnwindSafe(assert_nats_has_no_credentials(&url))
                    .catch_unwind()
                    .await
                    .is_err(),
                "credential surface was missed"
            );
            stream.delete_message(ack.sequence).await.unwrap();
        }
    }
    assert_nats_has_no_credentials(&url).await;
}

#[test]
fn state_scan_rejects_credentials_across_read_boundaries_and_in_sidecars() {
    for sentinel in CREDENTIAL_PATTERNS.iter() {
        for name in [
            "cooking.sqlite3",
            "cooking.sqlite3-wal",
            "cooking.sqlite3-shm",
        ] {
            let directory = tempfile::tempdir().expect("state scan fixture");
            std::fs::write(directory.path().join("cooking.sqlite3"), b"safe state")
                .expect("required snapshot file");
            let mut contents = vec![b'x'; 16_384 - 3];
            contents.extend_from_slice(sentinel);
            std::fs::write(directory.path().join(name), contents).expect("credential fixture");
            assert!(
                std::panic::catch_unwind(|| {
                    assert_state_snapshot_has_no_credentials(directory.path());
                })
                .is_err()
            );
        }
    }
    let directory = tempfile::tempdir().expect("safe scan fixture");
    std::fs::write(directory.path().join("cooking.sqlite3"), vec![b'x'; 32_768])
        .expect("safe snapshot");
    assert_state_snapshot_has_no_credentials(directory.path());
}
