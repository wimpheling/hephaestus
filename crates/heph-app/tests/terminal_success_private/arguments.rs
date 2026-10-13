//! Invalid caller arguments and separately committed negative OLD-result setup.
use super::{
    composition::{self, Baseline, Expected},
    fixture::{self, Fixture},
    observations, owning_transaction as owning,
};
use serde_json::{Value, json};
use vm_trait::VmExit;

#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_supplied_exit_is_required(pool: sqlx::PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    // Deliberately invalid caller arguments; the real controlled wait returned only exit0.
    let missing = VmExit {
        code: None,
        signal: None,
    };
    let positive = VmExit {
        code: Some(1),
        signal: None,
    };
    let negative = VmExit {
        code: Some(-1),
        signal: None,
    };
    let signal = VmExit {
        code: None,
        signal: Some(0),
    };
    let both = VmExit {
        code: Some(0),
        signal: Some(0),
    };
    let bytes = serde_json::to_vec(&f.birth.config.canonical_json()).unwrap();
    for (exit, failure) in [
        (None, None),
        (Some(&missing), None),
        (Some(&positive), None),
        (Some(&negative), None),
        (Some(&signal), None),
        (Some(&both), None),
        (Some(&prepared.exit), Some("")),
        (Some(&prepared.exit), Some("controlled failure")),
    ] {
        composition::deny_api(
            &f,
            &f.birth.repository(),
            exit,
            failure,
            Expected::Invalid(
                "Invocation success requires actual exit zero without signal or failure",
            ),
        )
        .await;
        composition::deny_facade(
            &f,
            composition::caller(&f, &bytes, exit, failure),
            "23000",
            composition::ARGUMENT,
        )
        .await;
    }
    let mut null_presence = composition::caller(&f, &bytes, Some(&prepared.exit), None);
    null_presence.present = None;
    composition::deny_facade(&f, null_presence, "23000", composition::ARGUMENT).await;
    prepared.unchanged(&f).await;
}

#[derive(Clone, Copy)]
enum Stored {
    Outcome,
    CodeZero,
    CodeOne,
    Signal,
    EmptyFailure,
    Failure,
}
impl Stored {
    const fn field(self) -> &'static str {
        match self {
            Self::Outcome => "outcome",
            Self::CodeZero | Self::CodeOne => "exit_code",
            Self::Signal => "exit_signal",
            Self::EmptyFailure | Self::Failure => "failure",
        }
    }
    fn value(self) -> Value {
        match self {
            Self::Outcome => json!("failed"),
            Self::CodeZero | Self::Signal => json!(0),
            Self::CodeOne => json!(1),
            Self::EmptyFailure => json!(""),
            Self::Failure => json!("controlled stored failure"),
        }
    }
}
async fn setup(f: &Fixture, field: Stored, clear: bool) {
    let before = observations::rows(f).await;
    let files = observations::files(f);
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    // These literal negative setup/reset writes are separate from every denied result transaction.
    let query = match (field, clear) {
        (Stored::Outcome, false) => sqlx::query("UPDATE runs SET outcome='failed' WHERE id=$1"),
        (Stored::Outcome, true) => sqlx::query("UPDATE runs SET outcome=NULL WHERE id=$1"),
        (Stored::CodeZero, false) => sqlx::query("UPDATE runs SET exit_code=0 WHERE id=$1"),
        (Stored::CodeOne, false) => sqlx::query("UPDATE runs SET exit_code=1 WHERE id=$1"),
        (Stored::CodeZero | Stored::CodeOne, true) => {
            sqlx::query("UPDATE runs SET exit_code=NULL WHERE id=$1")
        }
        (Stored::Signal, false) => sqlx::query("UPDATE runs SET exit_signal=0 WHERE id=$1"),
        (Stored::Signal, true) => sqlx::query("UPDATE runs SET exit_signal=NULL WHERE id=$1"),
        (Stored::EmptyFailure, false) => sqlx::query("UPDATE runs SET failure='' WHERE id=$1"),
        (Stored::Failure, false) => {
            sqlx::query("UPDATE runs SET failure='controlled stored failure' WHERE id=$1")
        }
        (Stored::EmptyFailure | Stored::Failure, true) => {
            sqlx::query("UPDATE runs SET failure=NULL WHERE id=$1")
        }
    };
    assert_eq!(
        query
            .bind(f.birth.start.run_id.as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    tx.commit().await.unwrap();
    let after = observations::rows(f).await;
    let mut expected = observations::run(&before, f);
    assert_eq!(expected["state"], "running");
    expected[field.field()] = if clear { Value::Null } else { field.value() };
    assert_eq!(
        observations::run(&after, f),
        expected,
        "only the deliberate negative field changes"
    );
    let mut expected_runs = before["runs"].clone();
    let stored_run = expected_runs
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["id"] == expected["id"])
        .unwrap();
    *stored_run = expected;
    expected_runs
        .as_array_mut()
        .unwrap()
        .sort_by_key(Value::to_string);
    assert_eq!(after["runs"], expected_runs);
    for (table, rows) in before.as_object().unwrap() {
        if ![
            "runs",
            "run_events",
            "outbox",
            "application_events",
            "product_event_outbox",
        ]
        .contains(&table.as_str())
        {
            assert_eq!(*rows, after[table], "negative setup preserves {table}");
        }
    }
    assert_eq!(observations::files(f), files);
    f.birth.physical.assert_unchanged();
}
async fn raw_result_denied(f: &Fixture, exit: &VmExit) {
    let before = Baseline::capture(f).await;
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    let error = sqlx::query(
        "UPDATE runs
         SET state = $2,
             outcome = COALESCE($3, outcome),
             exit_code = COALESCE($4, exit_code),
             exit_signal = COALESCE($5, exit_signal),
             failure = COALESCE($6, failure),
             updated_at = $7,
             state_version = state_version + 1
         WHERE id = $1",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .bind("succeeded")
    .bind(Some("succeeded"))
    .bind(exit.code)
    .bind(exit.signal)
    .bind(None::<&str>)
    .bind(crate::qualified_invocation::fixture::pg_time())
    .execute(&mut *tx)
    .await
    .unwrap_err();
    composition::sql_error(&error, "23000", composition::NEW_ROW);
    tx.rollback().await.unwrap();
    before.unchanged(f).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_old_result_cannot_prefill_success(pool: sqlx::PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    let bytes = serde_json::to_vec(&f.birth.config.canonical_json()).unwrap();
    for field in [
        Stored::Outcome,
        Stored::CodeZero,
        Stored::CodeOne,
        Stored::Signal,
        Stored::EmptyFailure,
        Stored::Failure,
    ] {
        setup(&f, field, false).await;
        // Capture all31 table snapshots/files immediately before each attempt, after seed commit.
        composition::deny_api(
            &f,
            &f.birth.repository(),
            Some(&prepared.exit),
            None,
            Expected::Storage("23000", composition::OLD_ROW),
        )
        .await;
        composition::deny_facade(
            &f,
            composition::caller(&f, &bytes, Some(&prepared.exit), None),
            "23000",
            composition::OLD_ROW,
        )
        .await;
        if matches!(field, Stored::CodeZero) {
            composition::deny_api(
                &f,
                &f.birth.repository(),
                None,
                None,
                Expected::Invalid(
                    "Invocation success requires actual exit zero without signal or failure",
                ),
            )
            .await;
            composition::deny_facade(
                &f,
                composition::caller(&f, &bytes, None, None),
                "23000",
                composition::ARGUMENT,
            )
            .await;
        }
        raw_result_denied(&f, &prepared.exit).await;
        setup(&f, field, true).await;
    }
    // Succeeded prefill is itself an early guarded negative, never committed setup.
    let before = Baseline::capture(&f).await;
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    let error = sqlx::query("UPDATE runs SET outcome='succeeded' WHERE id=$1")
        .bind(f.birth.start.run_id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap_err();
    composition::sql_error(&error, "23000", composition::NEW_ROW);
    tx.rollback().await.unwrap();
    before.unchanged(&f).await;
}
