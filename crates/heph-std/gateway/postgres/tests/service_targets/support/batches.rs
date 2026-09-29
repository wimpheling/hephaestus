//! Log append batch builders used by maintenance scenarios.

use gateway_domain::{GatewayServiceLogAppendBatch, ServiceLogLoss, ServiceLogRecord};
use time::OffsetDateTime;
use vm_trait::LogStream;

pub fn batch_with_sequence(sequence: u64) -> GatewayServiceLogAppendBatch {
    GatewayServiceLogAppendBatch::new(
        vec![ServiceLogRecord {
            sequence,
            stream: LogStream::Stdout,
            observed_at: OffsetDateTime::now_utc(),
            bytes: b"first durable line".to_vec(),
        }],
        ServiceLogLoss::default(),
    )
    .expect("valid bounded log batch")
}

pub fn batch_with_payload(sequence: u64, size: usize) -> GatewayServiceLogAppendBatch {
    GatewayServiceLogAppendBatch::new(
        vec![ServiceLogRecord {
            sequence,
            stream: LogStream::Stdout,
            observed_at: OffsetDateTime::now_utc(),
            bytes: vec![b'x'; size],
        }],
        ServiceLogLoss::default(),
    )
    .expect("valid bounded payload batch")
}

pub fn batch_with_payloads(start: u64, end: u64, size: usize) -> GatewayServiceLogAppendBatch {
    GatewayServiceLogAppendBatch::new(
        (start..=end)
            .map(|sequence| ServiceLogRecord {
                sequence,
                stream: LogStream::Stdout,
                observed_at: OffsetDateTime::now_utc(),
                bytes: vec![b'x'; size],
            })
            .collect(),
        ServiceLogLoss::default(),
    )
    .expect("valid bounded payload batch")
}
