use super::*;

#[cfg(feature = "test-fixtures")]
// Keep the complete SQL authority fixture together so its persisted scopes and
// application-role assertions remain reviewable as one setup boundary.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn prepare_gateway_service_log_rpc(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    project_id: uuid::Uuid,
) -> (
    control_plane_postgres::ControlPlanePool,
    GatewayServiceLogRpcFixture,
) {
    let composed_pool = running.application_pool_for_test();
    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&composed_pool)
        .await
        .expect("read composed service log RPC pool role");
    assert_eq!(current_user, "hephaestus_app");
    let error = sqlx::query("SELECT retained_bytes FROM gateway_service_log_project_usage")
        .fetch_optional(&composed_pool)
        .await
        .expect_err("application role cannot read worker-only project usage");
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code);
    assert_eq!(code.as_deref(), Some("42501"));
    let mut fixture_transaction = pool.begin().await.expect("begin service log RPC fixture");
    let (fencing_token, first_sequence): (i64, i64) = sqlx::query_as(
        "SELECT instance.fencing_token,
                COALESCE(MAX(chunks.sequence), -1) + 1
           FROM gateway_service_instances AS instance
           LEFT JOIN gateway_service_log_chunks AS chunks
             ON chunks.instance_id = instance.id
            AND chunks.gateway_id = instance.gateway_id
            AND chunks.revision_id = instance.revision_id
            AND chunks.fencing_token = instance.fencing_token
          WHERE instance.id = $1
            AND instance.gateway_id = $2
            AND instance.revision_id = $3
          GROUP BY instance.fencing_token",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_one(&mut *fixture_transaction)
    .await
    .expect("read ready service log RPC scope");
    assert!(fencing_token > 0);
    let baseline_epoch: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT acknowledged_through, retained_bytes, retained_chunks
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(project_id)
    .bind(fencing_token)
    .fetch_optional(&mut *fixture_transaction)
    .await
    .expect("read baseline service log RPC metadata");
    let (baseline_acknowledged, baseline_bytes, baseline_chunks) =
        baseline_epoch.unwrap_or((-1, 0, 0));

    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage (project_id)
         VALUES ($1)
         ON CONFLICT (project_id) DO NOTHING",
    )
    .bind(project_id)
    .execute(&mut *fixture_transaction)
    .await
    .expect("seed service log RPC usage");
    let inserted_epoch: Option<i64> = sqlx::query_scalar(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (instance_id, fencing_token) DO NOTHING
         RETURNING fencing_token",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(project_id)
    .bind(fencing_token)
    .fetch_optional(&mut *fixture_transaction)
    .await
    .expect("seed service log RPC epoch");
    if inserted_epoch.is_some() {
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_epochs = retained_epochs + 1, updated_at = now()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC epoch usage");
    }
    let payloads = [
        b"rpc-service-log-first".as_slice(),
        b"rpc-service-log-second".as_slice(),
    ];
    let payload_bytes = payloads
        .iter()
        .map(|payload| i64::try_from(payload.len()).expect("small log payload"))
        .sum::<i64>();
    for (offset, payload) in payloads.iter().enumerate() {
        let sequence = first_sequence + i64::try_from(offset).expect("small log sequence");
        sqlx::query(
            "INSERT INTO gateway_service_log_chunks
                (instance_id, gateway_id, revision_id, project_id, fencing_token,
                 sequence, stream, observed_at, bytes)
             VALUES ($1, $2, $3, $4, $5, $6, 'stdout', now(), $7)",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(project_id)
        .bind(fencing_token)
        .bind(sequence)
        .bind(*payload)
        .execute(&mut *fixture_transaction)
        .await
        .expect("seed service log RPC payload");
        sqlx::query(
            "UPDATE gateway_service_log_epochs
                SET acknowledged_through = GREATEST(acknowledged_through, $6),
                    retained_bytes = retained_bytes + $7,
                    retained_chunks = retained_chunks + 1,
                    updated_at = now()
              WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
                AND project_id = $4 AND fencing_token = $5",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(project_id)
        .bind(fencing_token)
        .bind(sequence)
        .bind(i64::try_from(payload.len()).expect("small log payload"))
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC metadata");
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_bytes = retained_bytes + $2,
                    retained_chunks = retained_chunks + 1,
                    storage_dropped_chunks = 7,
                    storage_dropped_bytes = 123,
                    updated_at = now()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .bind(i64::try_from(payload.len()).expect("small log payload"))
        .execute(&mut *fixture_transaction)
        .await
        .expect("update service log RPC project usage");
    }
    fixture_transaction
        .commit()
        .await
        .expect("commit service log RPC fixture");

    let foreign_owner = uuid::Uuid::new_v4();
    let foreign_organization = uuid::Uuid::new_v4();
    let foreign_project = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC Log Foreign Owner')")
        .bind(foreign_owner)
        .execute(pool)
        .await
        .expect("seed foreign service-log owner");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(foreign_organization)
        .bind(format!("rpc-log-foreign-{foreign_organization}"))
        .execute(pool)
        .await
        .expect("seed foreign service-log organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(foreign_organization)
    .bind(foreign_owner)
    .execute(pool)
    .await
    .expect("seed foreign service-log organization owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(foreign_project)
        .bind(foreign_organization)
        .bind(format!("rpc-log-foreign-project-{foreign_project}"))
        .execute(pool)
        .await
        .expect("seed foreign service-log project");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, storage_dropped_chunks, storage_dropped_bytes)
         VALUES ($1, 99, 999)",
    )
    .bind(foreign_project)
    .execute(pool)
    .await
    .expect("seed foreign service-log metadata");
    let member_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'RPC Log Member')")
        .bind(member_id)
        .execute(pool)
        .await
        .expect("seed service log RPC member");
    let member_browser_session = seed_golden_browser_session(
        pool,
        UserId::from_uuid(member_id),
        &golden_issuer(),
        &format!("member-{member_id}"),
    )
    .await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project_id)
        .bind(member_id)
        .execute(pool)
        .await
        .expect("grant service log RPC member");
    (
        composed_pool,
        GatewayServiceLogRpcFixture {
            gateway_id,
            revision_id,
            instance_id,
            project_id,
            fencing_token,
            first_sequence,
            baseline_acknowledged,
            baseline_bytes,
            baseline_chunks,
            payloads: [payloads[0].to_vec(), payloads[1].to_vec()],
            payload_bytes,
            foreign_project,
            member_id,
            member_browser_session,
        },
    )
}
