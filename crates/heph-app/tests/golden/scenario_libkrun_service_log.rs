#[cfg(feature = "test-fixtures")]
use crate::{
    GatewayServiceGoldenFixture, assert_guest_gateway_service_log_retained, cleanup_streams,
    gateway_service_log_rpc,
};
#[cfg(feature = "test-fixtures")]
use forge_domain::ProjectId;
#[cfg(feature = "test-fixtures")]
use identity_domain::{BrowserSessionSid, UserId};
#[cfg(feature = "test-fixtures")]
use std::{env, path::PathBuf};

#[cfg(feature = "test-fixtures")]
#[allow(clippy::too_many_arguments)]
pub async fn run_gateway_service_guest_log_phase(
    pool: &sqlx::PgPool,
    running: hephaestus_app::RunningHephaestus,
    service_fixture: &GatewayServiceGoldenFixture,
    first_instance_id: uuid::Uuid,
    first_resource_paths: &(PathBuf, PathBuf, PathBuf),
    project_id: ProjectId,
    user_id: UserId,
    owner_browser_session: &BrowserSessionSid,
    nats_url: &str,
) {
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("joined Caddy public URL for guest log proof");
    let fencing_token: i64 = sqlx::query_scalar(
        "SELECT fencing_token FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(first_instance_id)
    .bind(service_fixture.gateway_id)
    .bind(service_fixture.revision_id)
    .fetch_one(pool)
    .await
    .expect("read guest service-log fencing token");
    let guest_log_proof = gateway_service_log_rpc::exercise_gateway_service_guest_log(
        running.http_addr(),
        service_fixture.gateway_id,
        service_fixture.revision_id,
        first_instance_id,
        project_id.as_uuid(),
        fencing_token,
        user_id.as_uuid(),
        *owner_browser_session,
        &public_url,
    )
    .await;
    running
        .shutdown()
        .await
        .expect("guest service-log daemon shutdown");
    let first_state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(first_instance_id)
    .bind(service_fixture.gateway_id)
    .bind(service_fixture.revision_id)
    .fetch_optional(pool)
    .await
    .expect("read cleaned guest service-log instance");
    assert_eq!(
        first_state.as_deref(),
        Some("cleaned"),
        "guest service-log daemon shutdown must clean the service instance"
    );
    let (provider_runtime, cgroup, materializer) = first_resource_paths;
    assert!(!provider_runtime.exists());
    assert!(!cgroup.exists());
    assert!(!materializer.exists());
    assert_guest_gateway_service_log_retained(pool, &guest_log_proof).await;
    cleanup_streams(nats_url).await;
    println!(
        "REAL_GATEWAY_SERVICE_LOG_GUEST_E2E=1 instance={first_instance_id} fence={} retained_after_shutdown=true",
        guest_log_proof.fencing_token
    );
}
