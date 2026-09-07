//! Raw durable-storage checks for the disposable cooking database.

use base64::Engine as _;
use futures_util::TryStreamExt as _;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Read as _,
    path::Path,
    sync::LazyLock,
};

static CREDENTIAL_PATTERNS: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    super::cooking::FIXTURE_CREDENTIAL_SENTINELS
        .iter()
        .flat_map(|value| {
            let mut hexadecimal = String::with_capacity(value.len() * 2);
            for byte in value.as_bytes() {
                write!(hexadecimal, "{byte:02x}").expect("write fixture encoding");
            }
            [
                value.as_bytes().to_vec(),
                base64::engine::general_purpose::STANDARD
                    .encode(value)
                    .into_bytes(),
                hexadecimal.as_bytes().to_vec(),
                hexadecimal.to_ascii_uppercase().into_bytes(),
            ]
        })
        .collect()
});

/// Returns encoded fixture fingerprints for the in-process guest probe.
///
/// The caller receives a clone so the runtime observer can discard its copy
/// after each provider specification has been checked.
pub fn credential_patterns() -> Vec<Vec<u8>> {
    CREDENTIAL_PATTERNS.clone()
}

fn assert_bytes_have_no_credentials(bytes: &[u8]) {
    for pattern in CREDENTIAL_PATTERNS.iter() {
        assert!(
            !bytes.windows(pattern.len()).any(|window| window == pattern),
            "fixture credential encoding exposed"
        );
    }
}

/// Decode native VM log payloads while retaining overlap only within one
/// ordered run/stream. Never include payload contents in failure diagnostics.
#[derive(Default)]
struct VmLogScan {
    current: Option<(uuid::Uuid, String)>,
    suffix: Vec<u8>,
    byte_count: usize,
}

impl VmLogScan {
    fn inspect(&mut self, run: uuid::Uuid, stream: String, payload: &serde_json::Value) {
        assert!(
            matches!(stream.as_str(), "Stdout" | "Stderr"),
            "invalid VM log stream"
        );
        let values = payload.as_array().expect("VM log bytes must be an array");
        assert!(
            values.len() <= 1_048_576,
            "VM log chunk exceeds fixture bound"
        );
        let decoded: Vec<u8> = values
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .expect("VM log contains an invalid byte")
            })
            .collect();
        self.inspect_bytes(run, stream, &decoded);
    }

    fn inspect_bytes(&mut self, run: uuid::Uuid, stream: String, bytes: &[u8]) {
        let key = (run, stream);
        if self.current.as_ref() != Some(&key) {
            self.suffix.clear();
            self.current = Some(key);
        }
        assert!(
            bytes.len() <= 1_048_576,
            "stored log chunk exceeds fixture bound"
        );
        self.byte_count = self
            .byte_count
            .checked_add(bytes.len())
            .filter(|count| *count <= 256 * 1024 * 1024)
            .expect("stored log scan exceeds fixture bound");
        self.suffix.extend_from_slice(bytes);
        assert_bytes_have_no_credentials(&self.suffix);
        let longest = CREDENTIAL_PATTERNS
            .iter()
            .map(Vec::len)
            .max()
            .expect("fixture patterns");
        let discarded = self.suffix.len().saturating_sub(longest - 1);
        self.suffix.drain(..discarded);
    }
}

async fn assert_vm_logs_have_no_credentials(connection: &mut sqlx::PgConnection) {
    let mut rows = sqlx::query_as::<_, (uuid::Uuid, String, serde_json::Value)>(
        "SELECT run_id, payload->>'stream', payload->'bytes'
         FROM run_events WHERE event_type = 'vm.log'
         ORDER BY run_id, payload->>'stream', sequence",
    )
    .fetch(connection);
    let mut scan = VmLogScan::default();
    while let Some((run, stream, payload)) = rows.try_next().await.expect("read native VM logs") {
        scan.inspect(run, stream, &payload);
    }
    assert!(scan.byte_count > 0, "decoded VM log scan must not be empty");
    eprintln!(
        "Cooking decoded VM log credential scan: {} bytes",
        scan.byte_count
    );
}

/// Build logs are stored as ordered text chunks, separate from run events.
async fn assert_build_logs_have_no_credentials(connection: &mut sqlx::PgConnection) {
    let mut rows = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
        "SELECT execution.build_request_id, entry->>'stream', entry->>'text'
         FROM build_executions AS execution
         CROSS JOIN LATERAL jsonb_array_elements(execution.logs)
             WITH ORDINALITY AS logs(entry, ordinal)
         ORDER BY execution.build_request_id, entry->>'stream', ordinal",
    )
    .fetch(connection);
    let mut scan = VmLogScan::default();
    while let Some((build, stream, text)) = rows.try_next().await.expect("read stored build logs") {
        assert!(
            matches!(stream.as_str(), "stdout" | "stderr"),
            "invalid build log stream"
        );
        scan.inspect_bytes(build, stream, text.as_bytes());
    }
    assert!(
        scan.byte_count > 0,
        "stored build log scan must not be empty"
    );
    eprintln!(
        "Cooking stored build log credential scan: {} bytes",
        scan.byte_count
    );
}

/// Scan every retained `JetStream` message after the fixture daemon has stopped.
/// Acknowledged work-queue messages may already be deleted; this establishes
/// confinement for retained messages, not for previously consumed payloads.
pub async fn assert_nats_has_no_credentials(nats_url: &str) {
    let client = async_nats::connect(nats_url)
        .await
        .expect("connect for retained message scan");
    let context = async_nats::jetstream::new(client);
    let mut streams = context.streams();
    let mut surfaces = BTreeSet::new();
    let mut total = 0_u64;
    while let Some(info) = streams
        .try_next()
        .await
        .expect("enumerate retained streams")
    {
        let stream = context
            .get_stream(&info.config.name)
            .await
            .expect("open retained stream");
        let mut count = 0_u64;
        assert!(
            info.state.last_sequence < 100_000,
            "unexpected fixture stream size"
        );
        if info.state.messages > 0 {
            for sequence in info.state.first_sequence..=info.state.last_sequence {
                let message = match stream.get_raw_message(sequence).await {
                    Ok(message) => message,
                    Err(error)
                        if error.kind()
                            == async_nats::jetstream::stream::RawMessageErrorKind::NoMessageFound =>
                    {
                        continue;
                    }
                    Err(error) => panic!("cannot inspect retained stream message: {:?}", error.kind()),
                };
                assert_bytes_have_no_credentials(message.subject.as_bytes());
                assert_bytes_have_no_credentials(&message.payload);
                for (name, values) in message.headers.iter() {
                    let name: &str = name.as_ref();
                    assert_bytes_have_no_credentials(name.as_bytes());
                    for value in values {
                        assert_bytes_have_no_credentials(value.as_str().as_bytes());
                    }
                }
                count += 1;
            }
        }
        assert_eq!(
            count, info.state.messages,
            "retained stream changed during scan"
        );
        total += count;
        surfaces.insert(info.config.name);
    }
    for required in ["HEPH_RUN_COMMANDS", "HEPHAESTUS_PRODUCT_EVENTS"] {
        assert!(
            surfaces.contains(required),
            "missing required retained stream"
        );
    }
    assert!(total > 0, "retained message scan must not be empty");
    eprintln!(
        "Cooking retained NATS credential scan: {} streams, {total} messages",
        surfaces.len()
    );
}

/// Scan the extracted `SQLite` database and any WAL/shared-memory sidecars.
/// Keep a suffix between reads so a credential spanning chunks is detected.
pub fn assert_state_snapshot_has_no_credentials(directory: &Path) {
    let longest = CREDENTIAL_PATTERNS
        .iter()
        .map(Vec::len)
        .max()
        .expect("fixture credentials");
    assert!(longest > 0);
    for name in [
        "cooking.sqlite3",
        "cooking.sqlite3-wal",
        "cooking.sqlite3-shm",
    ] {
        let path = directory.join(name);
        if name != "cooking.sqlite3" && !path.exists() {
            continue;
        }
        assert!(
            path.is_file(),
            "state scan requires a regular snapshot file"
        );
        let mut file = std::fs::File::open(path).expect("open extracted state snapshot");
        let mut chunk = [0_u8; 16_384];
        let mut window = Vec::with_capacity(chunk.len() + longest);
        loop {
            let count = file
                .read(&mut chunk)
                .expect("read extracted state snapshot");
            if count == 0 {
                break;
            }
            window.extend_from_slice(&chunk[..count]);
            assert_bytes_have_no_credentials(&window);
            let retained = window.len().saturating_sub(longest - 1);
            window.drain(..retained);
        }
    }
}

/// Scan stored rows through the fixture administrator pool supplied by the
/// golden harness. This administrative storage scan is independent of tenant
/// projections and the runtime worker role, so every public application table
/// remains visible even when product grants intentionally exclude it. It checks
/// row representations, including JSON payloads, outboxes, and whole-value
/// hexadecimal/base64 encodings of the fixture credentials; it does
/// not stand in for guest-memory, network, or browser-evidence checks.
pub async fn assert_database_has_no_credentials(pool: &PgPool) {
    let mut transaction = pool.begin().await.expect("begin raw storage scan");
    let (role, superuser, bypass_rls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, r.rolsuper, r.rolbypassrls
         FROM pg_catalog.pg_roles AS r
         WHERE r.rolname = current_user",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("inspect fixture storage scan role");
    assert!(
        superuser || bypass_rls,
        "fixture storage scan requires rolsuper or rolbypassrls (current role: {role})"
    );
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('_sqlx_migrations', 'melange_migrations')
         ORDER BY c.relname",
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("enumerate fixture storage surfaces");
    let unreadable: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname
         FROM pg_catalog.pg_class AS c
         JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('_sqlx_migrations', 'melange_migrations')
           AND NOT has_table_privilege(current_user, c.oid, 'SELECT')
         ORDER BY c.relname",
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("check fixture storage surface visibility");
    assert!(
        unreadable.is_empty(),
        "fixture storage scan role cannot SELECT public tables: {}",
        unreadable.join(", ")
    );
    let patterns: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT pattern
         FROM unnest($1::text[]) AS fixture(value)
         CROSS JOIN LATERAL (VALUES
             (fixture.value),
             (encode(convert_to(fixture.value, 'UTF8'), 'hex')),
             (upper(encode(convert_to(fixture.value, 'UTF8'), 'hex'))),
             (replace(encode(convert_to(fixture.value, 'UTF8'), 'base64'), E'\\n', ''))
         ) AS encodings(pattern)",
    )
    .bind(super::cooking::FIXTURE_CREDENTIAL_SENTINELS.to_vec())
    .fetch_all(&mut *transaction)
    .await
    .expect("prepare transient credential scan patterns");
    // The query is static and reviewable. Compare its declared surfaces with
    // the live catalog so a new table cannot silently escape this scan.
    let covered: BTreeSet<_> = include_str!("confinement.sql")
        .lines()
        .filter_map(|line| line.strip_prefix("SELECT '")?.split('\'').next())
        .collect();
    let catalog: BTreeSet<_> = tables.iter().map(String::as_str).collect();
    assert_eq!(
        covered, catalog,
        "update static storage scan for schema changes"
    );
    let mut counts = BTreeMap::<String, u64>::new();
    let mut rows = sqlx::query_as::<_, (String, String)>(include_str!("confinement.sql"))
        .fetch(&mut *transaction);
    while let Some((surface, row)) = rows.try_next().await.expect("read raw fixture storage") {
        super::cooking::assert_no_credentials(&row);
        assert!(
            patterns.iter().all(|pattern| !row.contains(pattern)),
            "encoded fixture credential exposed in durable storage"
        );
        *counts.entry(surface).or_default() += 1;
    }
    drop(rows);
    for required in ["runs", "mailbox_events", "brokered_secret_lease_snapshots"] {
        assert!(
            counts.get(required).is_some_and(|count| *count > 0),
            "empty required scan surface: {required}"
        );
    }
    assert_vm_logs_have_no_credentials(&mut transaction).await;
    assert_build_logs_have_no_credentials(&mut transaction).await;
    let row_count: u64 = counts.values().sum();
    transaction
        .rollback()
        .await
        .expect("finish read-only storage scan");
    eprintln!(
        "Cooking raw database credential scan: {} tables, {row_count} rows",
        tables.len()
    );
}

#[cfg(test)]
mod tests {
    use super::assert_state_snapshot_has_no_credentials;
    use futures_util::FutureExt as _;

    #[test]
    fn native_vm_logs_reject_split_encodings_without_joining_streams() {
        let run = uuid::Uuid::new_v4();
        for pattern in super::CREDENTIAL_PATTERNS.iter() {
            let midpoint = pattern.len() / 2;
            let first = serde_json::json!(&pattern[..midpoint]);
            let second = serde_json::json!(&pattern[midpoint..]);
            let detected = std::panic::catch_unwind(|| {
                let mut scan = super::VmLogScan::default();
                scan.inspect(run, "Stdout".to_owned(), &first);
                scan.inspect(run, "Stdout".to_owned(), &second);
            });
            assert!(detected.is_err(), "split native log encoding escaped scan");
            let mut scan = super::VmLogScan::default();
            scan.inspect(run, "Stderr".to_owned(), &first);
            scan.inspect(run, "Stdout".to_owned(), &second);
            let mut scan = super::VmLogScan::default();
            scan.inspect(run, "Stdout".to_owned(), &first);
            scan.inspect(uuid::Uuid::new_v4(), "Stdout".to_owned(), &second);
        }
    }

    #[test]
    fn stored_build_logs_reject_split_text_encodings() {
        for pattern in super::CREDENTIAL_PATTERNS.iter() {
            let run = uuid::Uuid::new_v4();
            assert!(
                std::panic::catch_unwind(|| {
                    let mut scan = super::VmLogScan::default();
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
                    super::VmLogScan::default().inspect(
                        uuid::Uuid::new_v4(),
                        "Stdout".to_owned(),
                        &payload,
                    );
                })
                .is_err()
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires a dedicated disposable NATS server via HEPHAESTUS_CONFINEMENT_NATS_TEST_URL"]
    async fn retained_nats_scan_rejects_payload_header_and_subject_credentials() {
        let url = std::env::var("HEPHAESTUS_CONFINEMENT_NATS_TEST_URL")
            .expect("dedicated NATS fixture URL");
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
        super::assert_nats_has_no_credentials(&url).await;
        let stream = context
            .get_stream("HEPHAESTUS_PRODUCT_EVENTS")
            .await
            .unwrap();
        for pattern in super::CREDENTIAL_PATTERNS.iter() {
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
                    std::panic::AssertUnwindSafe(super::assert_nats_has_no_credentials(&url))
                        .catch_unwind()
                        .await
                        .is_err(),
                    "credential surface was missed"
                );
                stream.delete_message(ack.sequence).await.unwrap();
            }
        }
        super::assert_nats_has_no_credentials(&url).await;
    }

    #[test]
    fn state_scan_rejects_credentials_across_read_boundaries_and_in_sidecars() {
        for sentinel in super::CREDENTIAL_PATTERNS.iter() {
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
}
