use super::VmLogScan;
use futures_util::TryStreamExt as _;

pub(crate) async fn assert_vm_logs_have_no_credentials(connection: &mut sqlx::PgConnection) {
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
pub(crate) async fn assert_build_logs_have_no_credentials(connection: &mut sqlx::PgConnection) {
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
