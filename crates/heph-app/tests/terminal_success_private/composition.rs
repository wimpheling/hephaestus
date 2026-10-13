//! Exact adapter/facade denials and a bounded production-order rollback barrier.
use super::{
    fixture::{self, Fixture},
    observations, owning_transaction as owning,
};
use heph_run::{RunRepository, RunState};
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId};
use run_orchestrator::RepositoryError;
use run_postgres::PgRunRepository;
use serde_json::Value;
use std::collections::BTreeMap;
use vm_trait::VmExit;

pub const ARGUMENT: &str =
    "Invocation success requires actual caller VmExit0 without signal/failure";
pub const OLD_ROW: &str = "Invocation result differs from locked Running/configured provider owner";
pub const NEW_ROW: &str =
    "Invocation success needs exact Running/new result/closed acknowledged full set";
pub struct Baseline {
    rows: Value,
    files: BTreeMap<String, String>,
}
impl Baseline {
    pub async fn capture(f: &Fixture) -> Self {
        let rows = observations::rows(f).await;
        assert_eq!(rows.as_object().unwrap().len(), 31);
        Self {
            rows,
            files: observations::files(f),
        }
    }
    pub async fn unchanged(&self, f: &Fixture) {
        assert_eq!(observations::rows(f).await, self.rows);
        assert_eq!(observations::files(f), self.files);
        f.birth.physical.assert_unchanged();
    }
}
#[derive(Clone, Copy)]
pub enum Expected<'a> {
    Invalid(&'a str),
    Storage(&'a str, &'a str),
}
pub fn sql_error(error: &sqlx::Error, code: &str, message: &str) {
    let database = error.as_database_error().expect("exact database denial");
    assert_eq!(database.code().as_deref(), Some(code));
    assert_eq!(database.message(), message);
}
pub async fn deny_api(
    f: &Fixture,
    repository: &PgRunRepository,
    exit: Option<&VmExit>,
    failure: Option<&str>,
    expected: Expected<'_>,
) {
    let before = Baseline::capture(f).await;
    let error = repository
        .transition(f.birth.start.run_id, RunState::Succeeded, exit, failure)
        .await
        .unwrap_err();
    match (error, expected) {
        (RepositoryError::InvalidData(actual), Expected::Invalid(message)) => {
            assert_eq!(actual, message);
        }
        (RepositoryError::Storage(cause), Expected::Storage(code, message)) => {
            sql_error(
                cause
                    .downcast_ref::<sqlx::Error>()
                    .expect("original SQLx storage error"),
                code,
                message,
            );
        }
        (actual, _) => panic!("unexpected adapter denial: {actual:?}"),
    }
    rollback_barrier(f).await;
    before.unchanged(f).await;
}
#[derive(sqlx::FromRow)]
struct Reservation {
    lock_class: i32,
    command_mutex: Option<i64>,
    first_mutex: Option<i32>,
    second_mutex: Option<i32>,
}
async fn rollback_barrier(f: &Fixture) {
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let keys: Vec<Reservation> = sqlx::query_as(
        "SELECT * FROM invocation119_reservation_context($1) ORDER BY lock_class,command_mutex,first_mutex,second_mutex",
    ).bind(f.birth.start.run_id.as_uuid()).fetch_all(&mut *tx).await.unwrap();
    assert!(!keys.is_empty(), "complete original reservations");
    // Match production reservations119 exactly: blocking complete keys before any parent/child.
    for key in keys {
        match (
            key.lock_class,
            key.command_mutex,
            key.first_mutex,
            key.second_mutex,
        ) {
            (0, Some(command), None, None) => {
                sqlx::query("SELECT pg_advisory_xact_lock($1)")
                    .bind(command)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            }
            (1, None, Some(first), Some(second)) => {
                sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
                    .bind(first)
                    .bind(second)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            }
            _ => panic!("Invocation reservation tag or key shape differs"),
        }
    }
    let id: uuid::Uuid = sqlx::query_scalar(
        "SELECT execution.id FROM lock_qualified_invocation119($1,$2) execution",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .bind(serde_json::to_vec(&f.birth.config.canonical_json()).unwrap())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(id, f.birth.start.run_id.as_uuid());
    tx.rollback().await.unwrap();
}
#[derive(Clone, Copy)]
pub struct Caller<'a> {
    pub config: &'a [u8],
    pub namespace: Option<&'a str>,
    pub host: Option<&'a str>,
    pub present: Option<bool>,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub failure: Option<&'a str>,
}
pub fn caller<'a>(
    f: &'a Fixture,
    config: &'a [u8],
    exit: Option<&VmExit>,
    failure: Option<&'a str>,
) -> Caller<'a> {
    Caller {
        config,
        namespace: Some(f.birth.config.provider_scope().namespace()),
        host: Some(f.birth.config.provider_scope().host_id()),
        present: Some(exit.is_some()),
        code: exit.and_then(|e| e.code),
        signal: exit.and_then(|e| e.signal),
        failure,
    }
}
pub async fn deny_facade(f: &Fixture, input: Caller<'_>, code: &str, message: &str) {
    let before = Baseline::capture(f).await;
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    owning::configure(&mut tx).await;
    let error = sqlx::query_scalar::<_, String>(
        "SELECT qualify_invocation119_terminal_result($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(f.birth.start.run_id.as_uuid())
    .bind(input.config)
    .bind(input.namespace)
    .bind(input.host)
    .bind(input.present)
    .bind(input.code)
    .bind(input.signal)
    .bind(input.failure)
    .fetch_one(&mut *tx)
    .await
    .unwrap_err();
    sql_error(&error, code, message);
    tx.rollback().await.unwrap();
    before.unchanged(f).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires exact private28+selected29 PostgreSQL and genuine native Local births; controlled VM ports"]
async fn terminal_default_or_mismatched_composition_is_denied(pool: sqlx::PgPool) {
    let f = Box::pin(fixture::running(&pool, 2)).await;
    let prepared = owning::prepare(&f).await;
    let owned = PgRunRepository::new(f.birth.birth.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical);
    let configured = owned
        .clone()
        .with_vm_owner_scope(f.birth.scope.clone())
        .with_qualified_invocation_configuration(f.birth.config.clone());
    let wrong_namespace = RunCleanupHostId::new(
        "wrong-native-provider".into(),
        f.birth.scope.host_id().to_owned(),
    )
    .unwrap();
    let wrong_host = RunCleanupHostId::new(
        f.birth.scope.provider_namespace().to_owned(),
        "wrong-native-host".into(),
    )
    .unwrap();
    for (repository, message) in [
        (
            PgRunRepository::new(f.birth.birth.worker.clone()),
            "default composition does not support Invocation lifecycle",
        ),
        (
            configured.clone(),
            "Invocation lifecycle requires explicit119 composition",
        ),
        (
            owned
                .clone()
                .with_vm_owner_scope(f.birth.scope.clone())
                .with_qualified_invocation_lease_runtime(),
            "qualified Invocation configuration is absent",
        ),
        (
            owned
                .with_qualified_invocation_configuration(f.birth.config.clone())
                .with_qualified_invocation_lease_runtime(),
            "Invocation requires actual configured provider owner",
        ),
        (
            configured
                .clone()
                .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
                .with_qualified_invocation_lease_runtime(),
            "Invocation composition differs",
        ),
        (
            configured
                .clone()
                .with_vm_owner_scope(wrong_namespace)
                .with_qualified_invocation_lease_runtime(),
            "Invocation composition differs",
        ),
        (
            configured
                .with_vm_owner_scope(wrong_host)
                .with_qualified_invocation_lease_runtime(),
            "Invocation composition differs",
        ),
    ] {
        deny_api(
            &f,
            &repository,
            Some(&prepared.exit),
            None,
            Expected::Invalid(message),
        )
        .await;
    }
    let bytes = serde_json::to_vec(&f.birth.config.canonical_json()).unwrap();
    let mut different = bytes.clone();
    different.push(b' ');
    deny_facade(
        &f,
        caller(&f, &different, Some(&prepared.exit), None),
        "P1190",
        "complete consumed Invocation/config required",
    )
    .await;
    for (namespace, host) in [
        (
            Some("wrong-native-provider"),
            Some(f.birth.config.provider_scope().host_id()),
        ),
        (
            Some(f.birth.config.provider_scope().namespace()),
            Some("wrong-native-host"),
        ),
        (None, Some(f.birth.config.provider_scope().host_id())),
        (Some(f.birth.config.provider_scope().namespace()), None),
    ] {
        let mut input = caller(&f, &bytes, Some(&prepared.exit), None);
        input.namespace = namespace;
        input.host = host;
        deny_facade(&f, input, "23000", OLD_ROW).await;
    }
    prepared.unchanged(&f).await;
}
