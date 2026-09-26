use super::*;

fn record(sequence: u64) -> ServiceLogRecord {
    ServiceLogRecord {
        sequence,
        stream: LogStream::Stdout,
        observed_at: OffsetDateTime::UNIX_EPOCH,
        bytes: b"line".to_vec(),
    }
}

#[test]
fn append_batch_requires_strictly_ordered_bounded_records() {
    assert!(
        GatewayServiceLogAppendBatch::new(vec![record(0), record(1)], ServiceLogLoss::default())
            .is_ok()
    );
    assert!(
        GatewayServiceLogAppendBatch::new(vec![record(1), record(1)], ServiceLogLoss::default())
            .is_err()
    );
    assert!(
        GatewayServiceLogAppendBatch::new(vec![record(2), record(1)], ServiceLogLoss::default())
            .is_err()
    );
    assert!(
        GatewayServiceLogAppendBatch::new(
            vec![ServiceLogRecord {
                sequence: 0,
                stream: LogStream::Stdout,
                observed_at: OffsetDateTime::UNIX_EPOCH,
                bytes: vec![0; MAX_SERVICE_LOG_CHUNK_BYTES + 1],
            }],
            ServiceLogLoss::default()
        )
        .is_err()
    );
}
