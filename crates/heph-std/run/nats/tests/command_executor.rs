//! Actual broker transport tests; no VM, profile admission or DB proof.

#[path = "command_executor/control.rs"]
mod control;
#[path = "command_executor/fixture.rs"]
mod fixture;

use control::{ControlledExecutor, Mode};
use fixture::Fixture;
use futures_util::StreamExt;
use run_domain::CancelRun;
use run_nats::CommandHandlingError;
use run_nats::{CANCEL_RUN_SUBJECT, NatsCommandHandler, START_RUN_SUBJECT};
use run_orchestrator::OrchestratorError;
use runtime_types::CommandId;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires dedicated disposable JetStream account/server"]
async fn success_double_acknowledges_the_exact_start_command() {
    let mut fixture = Fixture::open(Duration::from_secs(30)).await;
    let executor = Arc::new(ControlledExecutor::new(Mode::Success));
    let handler = NatsCommandHandler::new_routed(executor.clone());
    let command = control::command();
    fixture
        .publish(START_RUN_SUBJECT, serde_json::to_vec(&command).unwrap())
        .await;
    let delivery = fixture.next().await;
    assert_eq!(delivery.info().unwrap().delivered, 1);
    handler.handle(&delivery).await.unwrap();
    assert_eq!(executor.starts(), vec![command]);
    assert!(executor.cancels().is_empty());
    fixture.wait_counts((0, 0)).await;
    drop(delivery);
    drop(handler);
    drop(executor);
    fixture.close().await;
    drop(fixture);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires dedicated disposable JetStream account/server"]
async fn failed_start_stays_unacknowledged_and_redelivers_identical_command() {
    // Only this disposable consumer uses a short redelivery lease; production
    // topology and its thirty-second acknowledgement deadline stay unchanged.
    let mut fixture = Fixture::open(Duration::from_millis(300)).await;
    let executor = Arc::new(ControlledExecutor::new(Mode::FailFirst));
    let handler = NatsCommandHandler::new_routed(executor.clone());
    let command = control::command();
    fixture
        .publish(START_RUN_SUBJECT, serde_json::to_vec(&command).unwrap())
        .await;
    let delivery = fixture.next().await;
    assert!(matches!(handler.handle(&delivery).await,
        Err(CommandHandlingError::Orchestration(OrchestratorError::RunInProgress(id)))
            if id == command.run_id));
    fixture.wait_counts((1, 1)).await;
    drop(delivery);
    let redelivery = fixture.next().await;
    assert!(redelivery.info().unwrap().delivered > 1);
    assert_eq!(
        serde_json::from_slice::<run_domain::StartRun>(&redelivery.payload).unwrap(),
        command
    );
    handler.handle(&redelivery).await.unwrap();
    assert_eq!(executor.starts(), vec![command.clone(), command]);
    fixture.wait_counts((0, 0)).await;
    drop(redelivery);
    drop(handler);
    drop(executor);
    fixture.close().await;
    drop(fixture);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires dedicated disposable JetStream account/server"]
async fn cancel_acknowledges_while_start_blocks_and_periodic_progress_keeps_start_pending() {
    let mut fixture = Fixture::open(Duration::from_secs(30)).await;
    let executor = Arc::new(ControlledExecutor::new(Mode::Blocked));
    let handler = NatsCommandHandler::new_routed(executor.clone());
    let consumer = fixture.consumer.clone();
    let task = tokio::spawn(async move { handler.serve(&consumer).await });
    let mut acknowledgements = fixture.acknowledgements().await;
    let command = control::command();
    fixture
        .publish(START_RUN_SUBJECT, serde_json::to_vec(&command).unwrap())
        .await;
    executor.wait_entered().await;
    let cancel = CancelRun {
        command_id: CommandId::new(),
        run_id: command.run_id,
        reason: "transport fixture".into(),
    };
    fixture
        .publish(CANCEL_RUN_SUBJECT, serde_json::to_vec(&cancel).unwrap())
        .await;
    executor.wait_cancelled().await;
    fixture.wait_counts((1, 1)).await;
    assert_eq!(executor.cancels(), vec![cancel]);
    assert_eq!(executor.starts(), vec![command]);
    let mut progress = 0;
    tokio::time::timeout(Duration::from_secs(15), async {
        while progress < 2 {
            let ack = acknowledgements
                .next()
                .await
                .expect("broker progress acknowledgement");
            if fixture.own_ack(&ack) && ack.payload.as_ref() == b"+WPI" {
                progress += 1;
            }
        }
    })
    .await
    .expect("initial and periodic ten-second progress ACKs");
    drop(acknowledgements);
    assert_eq!(fixture.counts().await, (1, 1));
    executor.release();
    fixture.wait_counts((0, 0)).await;
    drop(executor);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    fixture.close().await;
    drop(fixture);
}
