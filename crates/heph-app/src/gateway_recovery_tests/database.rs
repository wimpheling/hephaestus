use super::*;

pub(super) async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect real PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply gateway migrations");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest migration");
    let max_version = max_version.expect("migrations are present");
    assert!(max_version >= 76);
    println!("REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 max_migration={max_version}");
    Some(pool)
}

pub(super) struct IsolatedStartupDatabase {
    pub(super) control: sqlx::PgPool,
    pub(super) worker: sqlx::PgPool,
    pub(super) admin: sqlx::PgPool,
    pub(super) name: String,
}

#[derive(Clone, Copy)]
struct PoolTeardownState {
    size: u32,
    idle: usize,
    closed: bool,
}

impl fmt::Debug for PoolTeardownState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PoolTeardownState")
            .field("size", &self.size)
            .field("idle", &self.idle)
            .field("closed", &self.closed)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct IsolatedPoolTeardownState {
    control: PoolTeardownState,
    worker: PoolTeardownState,
    admin: PoolTeardownState,
}

impl fmt::Debug for IsolatedPoolTeardownState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IsolatedPoolTeardownState")
            .field("control", &self.control)
            .field("worker", &self.worker)
            .field("admin", &self.admin)
            .finish()
    }
}

#[derive(sqlx::FromRow)]
struct IsolatedSessionDiagnostic {
    pid: i32,
    application_name: String,
    state: String,
    backend_type: String,
    backend_start: Option<OffsetDateTime>,
    state_change: Option<OffsetDateTime>,
    wait_event_type: Option<String>,
    wait_event: Option<String>,
}

impl fmt::Debug for IsolatedSessionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IsolatedSessionDiagnostic")
            .field("pid", &self.pid)
            .field("application_name", &self.application_name)
            .field("state", &self.state)
            .field("backend_type", &self.backend_type)
            .field("backend_start", &self.backend_start)
            .field("state_change", &self.state_change)
            .field("wait_event_type", &self.wait_event_type)
            .field("wait_event", &self.wait_event)
            .finish()
    }
}

fn pool_teardown_state(pool: &sqlx::PgPool) -> PoolTeardownState {
    PoolTeardownState {
        size: pool.size(),
        idle: pool.num_idle(),
        closed: pool.is_closed(),
    }
}

fn assert_safe_isolated_database_name(name: &str) {
    assert!(name.starts_with("hephaestus_startup_"));
    assert!(name.len() > "hephaestus_startup_".len());
    assert!(
        name.bytes()
            .all(|byte| { byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' })
    );
}

pub(super) async fn isolated_startup_database() -> Option<IsolatedStartupDatabase> {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let options = PgConnectOptions::from_str(&database_url).expect("parse test database URL");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(
            options
                .clone()
                .database("postgres")
                .application_name("gateway-recovery-admin"),
        )
        .await
        .expect("connect PostgreSQL maintenance database");
    let name = format!("hephaestus_startup_{}", Uuid::new_v4().simple());
    assert_safe_isolated_database_name(&name);
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await
        .expect("create isolated startup database");
    let control = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(
            options
                .clone()
                .database(&name)
                .application_name("gateway-recovery-control"),
        )
        .await
        .expect("connect isolated startup database");
    sqlx::migrate!("../../migrations")
        .run(&control)
        .await
        .expect("migrate isolated startup database");
    let max_version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&control)
        .await
        .expect("read isolated migration version");
    let max_version = max_version.expect("isolated migrations");
    assert!(max_version >= 78);
    let worker = worker_pool_for_options(options.database(&name)).await;
    println!(
        "REAL_POSTGRES_CONNECTED_AND_MIGRATED=1 isolated_database={name} max_migration={max_version} worker_role=hephaestus_worker"
    );
    Some(IsolatedStartupDatabase {
        control,
        worker,
        admin,
        name,
    })
}

pub(super) async fn drop_isolated_startup_database(database: IsolatedStartupDatabase) {
    let before_close = IsolatedPoolTeardownState {
        control: pool_teardown_state(&database.control),
        worker: pool_teardown_state(&database.worker),
        admin: pool_teardown_state(&database.admin),
    };
    // SQLx 0.8.6 may have detached successful connection-return tasks that
    // began before close marked the pool closed. Drain both known pools on
    // every diagnostic iteration so a return completing after one pass is
    // handled by the next pass; this does not terminate a backend or relax
    // the session-free assertion below.
    let deadline = tokio::time::Instant::now() + StdDuration::from_secs(10);
    let mut after_close: Option<IsolatedPoolTeardownState> = None;
    // Pool::close completes client-side shutdown, but PostgreSQL can report a
    // terminated backend as idle briefly while it processes termination. Keep
    // this bounded timing-sensitive grace and the session diagnostic so the
    // fixture still asserts that the database becomes session-free.
    let mut last_sessions: Vec<IsolatedSessionDiagnostic> = Vec::new();
    loop {
        let close_result = tokio::time::timeout_at(deadline, async {
            database.control.close().await;
            database.worker.close().await;
        })
        .await;
        let current_pool_state = IsolatedPoolTeardownState {
            control: pool_teardown_state(&database.control),
            worker: pool_teardown_state(&database.worker),
            admin: pool_teardown_state(&database.admin),
        };
        after_close.get_or_insert(current_pool_state);
        assert!(
            close_result.is_ok(),
            "isolated startup pool close timed out; pools before close: {before_close:?}; pools after close: {current_pool_state:?}"
        );
        let sessions: Vec<IsolatedSessionDiagnostic> = match tokio::time::timeout_at(
            deadline,
            sqlx::query_as(
                "SELECT pid, coalesce(application_name, '') AS application_name,
                        coalesce(state, '') AS state,
                        coalesce(backend_type, '') AS backend_type,
                        backend_start, state_change, wait_event_type, wait_event
                   FROM pg_stat_activity
                  WHERE datname = $1 AND pid <> pg_backend_pid()
                  ORDER BY pid
                  LIMIT 32",
            )
            .bind(&database.name)
            .fetch_all(&database.admin),
        )
        .await
        {
            Ok(Ok(sessions)) => sessions,
            Ok(Err(error)) => panic!(
                "inspect isolated startup database sessions failed before teardown deadline: {error}; pools before close: {before_close:?}; pools after close: {after_close:?}; current pool state: {current_pool_state:?}; last sessions: {last_sessions:?}"
            ),
            Err(error) => panic!(
                "isolated startup database session inspection timed out: {error}; pools before close: {before_close:?}; pools after close: {after_close:?}; current pool state: {current_pool_state:?}; last sessions: {last_sessions:?}"
            ),
        };
        if sessions.is_empty() {
            break;
        }
        last_sessions = sessions;
        assert!(
            tokio::time::Instant::now() < deadline,
            "isolated startup database still has sessions after pool shutdown: {last_sessions:?}; pools before close: {before_close:?}; pools after close: {after_close:?}; current pool state: {current_pool_state:?}"
        );
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => {
                panic!("isolated startup database still has sessions after pool shutdown: {last_sessions:?}; pools before close: {before_close:?}; pools after close: {after_close:?}; current pool state: {current_pool_state:?}");
            }
            () = tokio::time::sleep(StdDuration::from_millis(25)) => {}
        }
    }
    assert_safe_isolated_database_name(&database.name);
    sqlx::query(&format!("DROP DATABASE IF EXISTS \"{}\"", database.name))
        .execute(&database.admin)
        .await
        .expect("drop isolated startup database");
    database.admin.close().await;
}

pub(super) async fn worker_pool() -> sqlx::PgPool {
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("worker test database URL");
    let options = PgConnectOptions::from_str(&database_url).expect("parse worker test URL");
    worker_pool_for_options(options).await
}

async fn worker_pool_for_options(options: PgConnectOptions) -> sqlx::PgPool {
    worker_pool_for_options_named(options, "gateway-recovery-test").await
}

pub(super) async fn worker_pool_for_options_named(
    options: PgConnectOptions,
    application_name: &'static str,
) -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SELECT set_config('application_name', $1, false)")
                    .bind(application_name)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .expect("connect worker PostgreSQL pool");
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .expect("read worker current user");
    assert_eq!(current_user, "hephaestus_worker");
    pool
}

// This real-PostgreSQL fixture deliberately builds the release, agent,
// revision, route, and leased instance graph in one place.
