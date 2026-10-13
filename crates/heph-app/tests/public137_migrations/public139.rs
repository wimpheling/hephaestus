//! Genuine installed139 role and absent/NULL-chain denials. Positive completed
//! Retain and authorized Reopen need the separate ordinary-public producer fixture.
use control_plane_postgres::verify_qualified_invocation_contract;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[derive(Clone)]
struct Pins {
    closure: Option<Uuid>,
    operation: Option<Uuid>,
    generation: Option<i64>,
    purpose: Option<Vec<u8>>,
    namespace: Option<String>,
    host: Option<String>,
    retention: Option<Uuid>,
}

impl Pins {
    fn missing() -> Self {
        Self {
            closure: Some(Uuid::new_v4()),
            operation: Some(Uuid::new_v4()),
            generation: Some(1),
            purpose: Some(vec![19; 32]),
            namespace: Some("public139-test-provider".into()),
            host: Some("public139-test-host".into()),
            retention: Some(Uuid::new_v4()),
        }
    }

    fn null_slot(&mut self, slot: usize) {
        match slot {
            0 => self.closure = None,
            1 => self.operation = None,
            2 => self.generation = None,
            3 => self.purpose = None,
            4 => self.namespace = None,
            5 => self.host = None,
            6 => self.retention = None,
            _ => panic!("unknown139 pin slot"),
        }
    }
}

pub(super) async fn assert_worker_contract(admin: &PgPool) -> Result<(), sqlx::Error> {
    let worker = role_pool(admin, true).await?;
    let application = match role_pool(admin, false).await {
        Ok(pool) => pool,
        Err(error) => {
            worker.close().await;
            return Err(error);
        }
    };
    let result = inspect(admin, &worker, &application).await;
    worker.close().await;
    application.close().await;
    result
}

async fn inspect(admin: &PgPool, worker: &PgPool, application: &PgPool) -> Result<(), sqlx::Error> {
    verify_qualified_invocation_contract(admin, worker, application).await?;
    assert!(
        verify_qualified_invocation_contract(admin, admin, application)
            .await
            .is_err()
    );
    assert!(
        verify_qualified_invocation_contract(admin, worker, worker)
            .await
            .is_err()
    );
    let expected = [
        (admin, "postgres", "none", "42501"),
        (application, "hephaestus_app", "hephaestus_app", "42501"),
        (worker, "hephaestus_worker", "hephaestus_worker", "23000"),
    ];
    let missing = Pins::missing();
    for (pool, role, setting, code) in expected {
        deny(pool, &missing, role, setting, code).await?;
    }
    for slot in 0..7 {
        let mut null = missing.clone();
        null.null_slot(slot);
        deny(
            worker,
            &null,
            "hephaestus_worker",
            "hephaestus_worker",
            "23000",
        )
        .await?;
    }
    Ok(())
}

async fn deny(
    pool: &PgPool,
    pins: &Pins,
    role: &str,
    setting: &str,
    code: &str,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let result = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let actual: (String, String, String) =
            sqlx::query_as("SELECT session_user::text,current_user::text,current_setting('role')")
                .fetch_one(&mut *transaction)
                .await?;
        let outcome =
            sqlx::query("SELECT public.assert_retained_owned_reuse_context($1,$2,$3,$4,$5,$6,$7)")
                .bind(pins.closure)
                .bind(pins.operation)
                .bind(pins.generation)
                .bind(pins.purpose.as_deref())
                .bind(pins.namespace.as_deref())
                .bind(pins.host.as_deref())
                .bind(pins.retention)
                .execute(&mut *transaction)
                .await;
        Ok::<_, sqlx::Error>((actual, outcome))
    }
    .await;
    // Roll back before checking expected denial; preserve setup/query primary errors.
    let rollback = transaction.rollback().await;
    let (actual, outcome) = result?;
    rollback?;
    assert_eq!(actual, ("postgres".into(), role.into(), setting.into()));
    let error = outcome.expect_err("no absent or NULL chain can qualify");
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some(code)
    );
    Ok(())
}

async fn role_pool(admin: &PgPool, worker: bool) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if worker {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                } else {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(admin.connect_options().as_ref().clone())
        .await
}
