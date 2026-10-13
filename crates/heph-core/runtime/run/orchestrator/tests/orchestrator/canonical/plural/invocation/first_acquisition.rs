//! Controlled Core ordering only; facade SQL/real permissions have separate gates.
#[path = "first_acquisition/fixture.rs"]
mod fixture;
#[path = "first_acquisition/ports.rs"]
mod ports;
#[path = "first_acquisition/store.rs"]
mod store;

use crate::{canonical::Fixture, support::lock};
use fixture::Controlled;
use run_domain::{RunKind, RunState};
use std::{sync::atomic::Ordering, time::Duration};

#[tokio::test]
async fn invocation_first_acquisition_defers_only_secret_check_until_actual_provisioning() {
    for count in [0, 2] {
        let c = Controlled::new(count).await;
        c.complete().await;
        let log = lock(&c.fixture.log).clone();
        let index = |name| log.iter().position(|item| *item == name).unwrap();
        let ordering = [
            "first-selection",
            "first-launch",
            "first-authority",
            "first-mount",
            "first-acquire",
            "provisioning-secret",
            "first-runtime",
            "first-secret-prepare",
            "first-authority-prepare",
            "provision",
            "start",
            "first-ack",
            "first-attach",
        ];
        for pair in ordering.windows(2) {
            assert!(index(pair[0]) < index(pair[1]), "{pair:?}: {log:?}");
        }
        assert_eq!(c.fixture.count("leasing-secret"), 0);
        assert_eq!(c.fixture.count("first-acquire"), 1);
        assert_eq!(c.fixture.runs.run.lock().await.state, RunState::CleanedUp);
    }
}

#[tokio::test]
async fn invocation_first_acquisition_requires_configuration_current_owner_and_launch_port() {
    for count in [0, 2] {
        for (configured, launch, substitute_owner) in [
            (false, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            let c = Controlled::new(count).await;
            c.store
                .substitute
                .store(u8::from(substitute_owner), Ordering::SeqCst);
            assert!(
                c.runner(configured, launch)
                    .start_run(&c.fixture.command)
                    .await
                    .is_err()
            );
            c.no_acquisition_or_materialization();
            assert_eq!(c.fixture.count("leasing-secret"), 0);
        }
    }
}

#[tokio::test]
async fn invocation_first_acquisition_rejects_substituted_persisted_phase_and_pins() {
    for mode in 2..=8 {
        let c = Controlled::new(2).await;
        c.store.substitute.store(mode, Ordering::SeqCst);
        assert!(
            c.runner(true, true)
                .start_run(&c.fixture.command)
                .await
                .is_err()
        );
        c.no_acquisition_or_materialization();
        assert_eq!(c.fixture.count("first-launch"), 0);
    }
}

#[tokio::test]
async fn invocation_first_acquisition_rejects_existing_scalar_tuple_and_cancellation() {
    for count in [0, 2] {
        for mode in 0..4 {
            let c = Controlled::new(count).await;
            let mut run = c.fixture.runs.run.lock().await;
            // These fields are present in BOTH supplied and reread projections;
            // equality alone cannot reject an already contradictory tuple.
            match mode {
                0 => run.volume_id = Some(runtime_types::VolumeId::new()),
                1 => run.lease_id = Some(runtime_types::LeaseId::new()),
                2 => run.lease_fencing_token = Some(1),
                _ => run.cancel_requested_at = Some(time::OffsetDateTime::now_utc()),
            }
            drop(run);
            assert!(
                c.runner(true, true)
                    .start_run(&c.fixture.command)
                    .await
                    .is_err()
            );
            c.no_acquisition_or_materialization();
            assert_eq!(c.fixture.count("first-launch"), 0);
        }
    }
}

#[tokio::test]
async fn invocation_first_acquisition_retains_launch_authority_and_whole_mount_refusals() {
    for count in [0, 2] {
        for mode in 0..3 {
            let c = Controlled::new(count).await;
            c.ports.deny_launch.store(mode == 0, Ordering::SeqCst);
            c.ports.deny_authority.store(mode == 1, Ordering::SeqCst);
            c.store
                .inner
                .mount_denied
                .store(mode == 2, Ordering::SeqCst);
            assert!(
                c.runner(true, true)
                    .start_run(&c.fixture.command)
                    .await
                    .is_err()
            );
            c.no_acquisition_or_materialization();
            assert_eq!(c.fixture.count("first-launch"), 1);
            assert_eq!(c.fixture.count("first-authority"), usize::from(mode != 0));
            assert_eq!(c.fixture.count("first-mount"), usize::from(mode == 2));
            assert_eq!(c.fixture.count("leasing-secret"), 0);
        }
    }
}

#[tokio::test]
async fn invocation_provisioning_secret_refusal_precedes_runtime_and_provider_even_without_slots() {
    for count in [0, 2] {
        let c = Controlled::new(count).await;
        // Models a missing/default or contradictory B2 verifier, which refuses
        // at its actual persisted Provisioning boundary, not a Core skip flag.
        c.ports.deny_secrets.store(true, Ordering::SeqCst);
        let result = c.runner(true, true).start_run(&c.fixture.command).await;
        let run = match result {
            Ok(run) => run,
            Err(_) => c.fixture.runs.run.lock().await.clone(),
        };
        assert_eq!(run.outcome, Some(run_domain::RunOutcome::Failed));
        assert_eq!(c.fixture.count("first-acquire"), 1);
        assert_eq!(c.fixture.count("leasing-secret"), 0);
        assert_eq!(c.fixture.count("provisioning-secret"), 1);
        for name in [
            "first-runtime",
            "first-secret-prepare",
            "first-authority-prepare",
            "provision",
            "start",
        ] {
            assert_eq!(c.fixture.count(name), 0, "{name}");
        }
    }
}

#[tokio::test]
async fn normal_and_update_keep_initial_secret_reauthorization_before_acquisition() {
    for kind in [RunKind::Normal, RunKind::Update] {
        for count in [0, 2] {
            let mut f = Fixture::new();
            f.command.kind = kind;
            if kind == RunKind::Update {
                f.command.attachment_id = None;
            }
            let mut run = f.runs.run.lock().await;
            run.kind = kind;
            run.attachment_id = f.command.attachment_id;
            drop(run);
            let c = Controlled::from_fixture(f, count);
            c.ports.deny_secrets.store(true, Ordering::SeqCst);
            assert!(
                c.runner(false, true)
                    .start_run(&c.fixture.command)
                    .await
                    .is_err()
            );
            c.no_acquisition_or_materialization();
            assert_eq!(c.fixture.count("leasing-secret"), 1);
            assert_eq!(c.fixture.count("first-authority"), 0);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn invocation_first_acquisition_retains_one_combined_two_second_deadline() {
    let c = Controlled::new(0).await;
    c.ports.delayed.store(true, Ordering::SeqCst);
    let before = tokio::time::Instant::now();
    assert!(
        c.runner(true, true)
            .start_run(&c.fixture.command)
            .await
            .is_err()
    );
    assert_eq!(before.elapsed(), Duration::from_secs(2));
    assert_eq!(c.fixture.count("first-launch"), 1);
    assert_eq!(c.fixture.count("first-authority"), 1);
    c.no_acquisition_or_materialization();
}
