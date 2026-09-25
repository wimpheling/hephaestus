use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome,
    GatewayServiceLogStore, GatewayServiceLogStoreError, GatewayServiceOwner,
    ServiceLogBufferHandle,
};
use ::time::OffsetDateTime;
use async_trait::async_trait;
use tokio::time::{self, Duration, Instant};
use uuid::Uuid;
use vm_trait::LogStream;

use super::*;

struct FakeStore {
    results: Mutex<VecDeque<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>>,
    batches: Mutex<Vec<GatewayServiceLogAppendBatch>>,
    delay_millis: Arc<AtomicU64>,
}

#[async_trait]
impl GatewayServiceLogStore for FakeStore {
    async fn append_batch(
        &self,
        _lease: &GatewayServiceInstanceLease,
        _owner: &GatewayServiceOwner,
        batch: GatewayServiceLogAppendBatch,
    ) -> Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError> {
        self.batches.lock().expect("batches lock").push(batch);
        let delay_millis = self.delay_millis.load(Ordering::Relaxed);
        if delay_millis != 0 {
            time::sleep(Duration::from_millis(delay_millis)).await;
        }
        self.results
            .lock()
            .expect("results lock")
            .pop_front()
            .unwrap_or_else(|| Ok(GatewayServiceLogAppendOutcome::default()))
    }
}

fn identity() -> GatewayServiceInstanceLease {
    GatewayServiceInstanceLease {
        identity: crate::GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        owner_host_id: "test-host".to_owned(),
        owner_uuid: Uuid::new_v4(),
        fencing_token: 1,
        state: crate::GatewayServiceInstanceState::Ready,
        vm_id: "test-vm".to_owned(),
        lease_expires_at: OffsetDateTime::now_utc() + ::time::Duration::hours(1),
        heartbeat_at: OffsetDateTime::now_utc(),
    }
}

fn writer(
    results: Vec<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>,
) -> (
    ServiceLogWriter<FakeStore>,
    Arc<FakeStore>,
    ServiceLogBufferHandle,
) {
    writer_with_delay(results, Duration::ZERO)
}

fn writer_with_delay(
    results: Vec<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>,
    delay: Duration,
) -> (
    ServiceLogWriter<FakeStore>,
    Arc<FakeStore>,
    ServiceLogBufferHandle,
) {
    let lease = identity();
    let owner =
        GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid).expect("owner");
    let store = Arc::new(FakeStore {
        results: Mutex::new(results.into_iter().collect()),
        batches: Mutex::new(Vec::new()),
        delay_millis: Arc::new(AtomicU64::new(
            u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
        )),
    });
    let buffer = ServiceLogBufferHandle::new();
    let writer = ServiceLogWriter::new(
        buffer.clone(),
        Arc::clone(&store),
        lease,
        owner,
        ServiceLogWriterPolicy::new(
            Duration::from_millis(20),
            Duration::from_millis(5),
            Duration::from_millis(20),
        ),
    )
    .expect("writer");
    (writer, store, buffer)
}

#[tokio::test]
async fn appends_one_bounded_batch_and_loss_only_updates_are_not_duplicated() {
    let (mut writer, store, buffer) = writer(vec![Ok(GatewayServiceLogAppendOutcome {
        accepted_chunks: 1,
        ..GatewayServiceLogAppendOutcome::default()
    })]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"one");
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
    buffer.record_provider_lag(3);
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
    assert!(matches!(writer.poll().await, ServiceLogWriterPoll::Idle));
    let batches = store.batches.lock().expect("batches lock");
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].records.len(), 1);
    assert_eq!(batches[1].records.len(), 0);
    assert_eq!(batches[1].loss.provider_lagged_events, 3);
    drop(batches);
}

#[tokio::test]
async fn unavailable_keeps_same_batch_and_backoff_is_bounded() {
    let (mut writer, store, buffer) = writer(vec![
        Err(GatewayServiceLogStoreError::Unavailable),
        Ok(GatewayServiceLogAppendOutcome::default()),
    ]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"same");
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::RetryScheduled { .. }
    ));
    assert_eq!(writer.snapshot().queued_chunks, 0);
    time::sleep(Duration::from_millis(6)).await;
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
    let batches = store.batches.lock().expect("batches lock");
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[0].records[0].sequence,
        batches[1].records[0].sequence
    );
    assert_eq!(batches[0].records[0].bytes, batches[1].records[0].bytes);
    drop(batches);
}

#[tokio::test]
async fn dropped_poll_retains_batch_and_final_flush_reports_deadline() {
    let (mut first_writer, _store, buffer) =
        writer(vec![Err(GatewayServiceLogStoreError::Unavailable)]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"pending");
    let poll = tokio::spawn(async move { first_writer.poll().await });
    let _ = poll.await.expect("poll");
    let (mut writer_instance, _store, buffer) =
        writer(vec![Err(GatewayServiceLogStoreError::Unavailable)]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"pending");
    let _ = writer_instance.poll().await;
    let flush = writer_instance
        .final_flush(Instant::now() + Duration::from_millis(2))
        .await;
    assert!(!flush.complete);
    assert_eq!((flush.unflushed_chunks, flush.unflushed_bytes), (1, 7));
}

#[tokio::test]
async fn cancelling_in_flight_append_retains_the_same_batch_for_retry() {
    let (mut writer, store, buffer) = writer_with_delay(
        vec![Ok(GatewayServiceLogAppendOutcome::default())],
        Duration::from_millis(50),
    );
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"retry");
    assert!(
        time::timeout(Duration::from_millis(2), writer.poll())
            .await
            .is_err()
    );
    store.delay_millis.store(0, Ordering::Relaxed);
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
    let batches = store.batches.lock().expect("batches lock");
    assert_eq!(batches.len(), 2);
    assert_eq!(
        batches[0].records[0].sequence,
        batches[1].records[0].sequence
    );
    assert_eq!(batches[0].records[0].bytes, batches[1].records[0].bytes);
    drop(batches);
}

#[tokio::test]
async fn capacity_discards_only_one_batch_and_allows_later_records() {
    let (mut writer, _store, buffer) = writer(vec![
        Err(GatewayServiceLogStoreError::Capacity),
        Ok(GatewayServiceLogAppendOutcome::default()),
    ]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"drop");
    assert_eq!(
        writer.poll().await,
        ServiceLogWriterPoll::Capacity {
            discarded_chunks: 1,
            discarded_bytes: 4
        }
    );
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"keep");
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
}

#[tokio::test]
async fn deadline_keeps_empty_loss_batch_for_later_retry() {
    let (mut writer, store, buffer) = writer_with_delay(
        vec![Ok(GatewayServiceLogAppendOutcome::default())],
        Duration::from_millis(50),
    );
    buffer.record_provider_lag(1);
    let flush = writer
        .final_flush(Instant::now() + Duration::from_millis(2))
        .await;
    assert!(!flush.complete);
    assert_eq!(flush.unflushed_chunks, 0);
    assert_eq!(flush.unflushed_bytes, 0);
    assert_eq!(flush.unflushed_loss.provider_lagged_events, 1);
    assert!(flush.terminal_error.is_none());

    store.delay_millis.store(0, Ordering::Relaxed);
    assert!(matches!(
        writer.poll().await,
        ServiceLogWriterPoll::Appended { .. }
    ));
    let completed = writer
        .final_flush(Instant::now() + Duration::from_millis(10))
        .await;
    assert!(completed.complete);
}

#[tokio::test]
async fn terminal_results_stop_without_retrying() {
    let (mut writer, store, buffer) = writer(vec![Err(GatewayServiceLogStoreError::StaleLease)]);
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"gone");
    assert_eq!(
        writer.poll().await,
        ServiceLogWriterPoll::Terminated {
            error: Some(GatewayServiceLogStoreError::StaleLease)
        }
    );
    assert_eq!(
        writer.poll().await,
        ServiceLogWriterPoll::Terminated {
            error: Some(GatewayServiceLogStoreError::StaleLease)
        }
    );
    assert_eq!(store.batches.lock().expect("batches lock").len(), 1);
}
