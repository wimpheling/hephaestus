use crate::{FlushDiagnostics, FlushPublisher, flush_publisher, flush_until_quiescent};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[tokio::test]
async fn final_outbox_flush_drains_beyond_the_old_pass_cap() {
    let mut passes = 0_u16;
    let deadline = Instant::now() + Duration::from_secs(1);
    let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
    flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
        passes += 1;
        let quiescent = passes > 100;
        async move { Ok(quiescent) }
    })
    .await
    .expect("flush reaches quiescence after more than 100 passes");
    assert_eq!(passes, 101);
}

#[tokio::test]
async fn final_outbox_flush_preserves_deadline_failure() {
    let called = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .expect("deadline remains representable");
    let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
    let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), {
        let called = Arc::clone(&called);
        move || {
            called.store(true, Ordering::Release);
            async { Ok(false) }
        }
    })
    .await;
    assert!(matches!(
        result,
        Err(crate::AppError::Shutdown(message))
            if message == "final outbox flush did not quiesce"
    ));
    assert!(!called.load(Ordering::Acquire));
    let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
    assert!(diagnostics.deadline_expired_before_pass);
    assert_eq!(diagnostics.passes, 1);
    drop(diagnostics);
}

#[tokio::test]
async fn final_outbox_flush_diagnoses_an_interrupted_publisher() {
    // This crate does not enable Tokio's test clock; leave enough real time for
    // the publisher phase to be entered before the pending operation times out.
    let deadline = Instant::now() + Duration::from_millis(250);
    let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
    let pass_diagnostics = Arc::clone(&diagnostics);
    let result = flush_until_quiescent(deadline, Arc::clone(&diagnostics), || {
        let diagnostics = Arc::clone(&pass_diagnostics);
        async move {
            let forge = flush_publisher(
                &diagnostics,
                FlushPublisher::Forge,
                async { Ok::<usize, std::io::Error>(3) },
                "test forge outbox flush",
            )
            .await?;
            assert_eq!(forge, 3);
            let _ = flush_publisher(
                &diagnostics,
                FlushPublisher::ProductEvent,
                std::future::pending::<Result<usize, std::io::Error>>(),
                "test product-event outbox flush",
            )
            .await?;
            Ok(false)
        }
    })
    .await;
    assert!(matches!(
        result,
        Err(crate::AppError::Shutdown(message))
            if message == "final outbox flush did not quiesce"
    ));
    let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
    assert!(diagnostics.deadline_expired_during_pass);
    assert!(diagnostics.deadline_expired_during_publisher);
    assert_eq!(
        diagnostics.active.map(|phase| phase.publisher.name()),
        Some("product-event")
    );
    assert_eq!(
        diagnostics.last_batches[FlushPublisher::Forge.index()],
        Some(3)
    );
    assert_eq!(
        diagnostics.last_batches[FlushPublisher::ProductEvent.index()],
        None
    );
    assert_eq!(diagnostics.failure_kind, Some("deadline-during-publisher"));
    drop(diagnostics);
}

#[tokio::test]
async fn final_outbox_flush_classifies_publisher_errors() {
    let deadline = Instant::now() + Duration::from_secs(1);
    let diagnostics = Arc::new(std::sync::Mutex::new(FlushDiagnostics::new(deadline)));
    let result = flush_publisher(
        &diagnostics,
        FlushPublisher::ProductEvent,
        async { Err::<usize, _>(std::io::Error::other("publisher unavailable")) },
        "test product-event outbox flush",
    )
    .await;
    assert!(matches!(
        result,
        Err(crate::AppError::Component {
            component: "test product-event outbox flush",
            ..
        })
    ));
    let diagnostics = diagnostics.lock().expect("flush diagnostics mutex");
    assert_eq!(diagnostics.failure_kind, Some("publisher-error"));
    assert!(diagnostics.active.is_none());
    assert_eq!(
        diagnostics.last_phase.map(|phase| phase.publisher.name()),
        Some("product-event")
    );
    drop(diagnostics);
}
