//! Release UI authorization fixture seeding.

use identity_domain::UserId;
use sqlx::PgPool;
use uuid::Uuid;

use super::helpers::{
    ReleaseFixture, publish, seed_agent, seed_artifact, seed_descriptor,
    seed_identity_and_repository, seed_release, seed_ui_source,
};

#[derive(Clone)]
pub(super) struct Seeded {
    pub(super) owner: UserId,
    pub(super) outsider: UserId,
    pub(super) valid: ReleaseFixture,
    pub(super) legacy: ReleaseFixture,
    pub(super) missing_managed: ReleaseFixture,
    pub(super) non_html: ReleaseFixture,
    pub(super) overflow: ReleaseFixture,
    pub(super) file_overflow: ReleaseFixture,
    pub(super) api_overflow: ReleaseFixture,
    pub(super) html_artifact: Uuid,
    pub(super) agent_id: Uuid,
}

#[allow(clippy::too_many_lines)] // Keep the complete malformed-row fixture matrix in one seed phase.
#[allow(clippy::cognitive_complexity)] // Preserve the fixture order that mirrors the authorization cases.
pub(super) async fn seed_all(pool: &PgPool) -> Seeded {
    let owner = UserId::new();
    let outsider = UserId::new();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    seed_identity_and_repository(pool, owner, outsider, organization, project, repository).await;

    let valid = seed_release(pool, owner, repository, 1).await;
    let agent_id = seed_agent(pool, repository, valid.release_id, 1).await;
    let html_artifact = seed_artifact(pool, valid.release_id, 1, "text/html").await;
    seed_ui_source(pool, &valid).await;
    seed_descriptor(
        pool,
        valid.release_id,
        "docs",
        "static",
        "docs",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(valid.release_id)
    .bind(html_artifact)
    .execute(pool)
    .await
    .expect("seed static UI file");
    seed_descriptor(
        pool,
        valid.release_id,
        "service",
        "managed_service",
        "service",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'gateway', '/service', $2)",
    )
    .bind(valid.release_id)
    .bind(agent_id)
    .execute(pool)
    .await
    .expect("seed managed UI service");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', 'health', 'gateway', 'GET', '/health', $2)",
    )
    .bind(valid.release_id)
    .bind(agent_id)
    .execute(pool)
    .await
    .expect("seed UI API binding");
    publish(pool, valid.release_id, owner).await;

    let legacy = seed_release(pool, owner, repository, 2).await;
    publish(pool, legacy.release_id, owner).await;

    let missing_managed = seed_release(pool, owner, repository, 3).await;
    seed_ui_source(pool, &missing_managed).await;
    seed_descriptor(
        pool,
        missing_managed.release_id,
        "missing",
        "managed_service",
        "missing",
        "index.html",
    )
    .await;
    publish(pool, missing_managed.release_id, owner).await;

    let non_html = seed_release(pool, owner, repository, 4).await;
    let plain_artifact = seed_artifact(pool, non_html.release_id, 4, "text/plain").await;
    seed_ui_source(pool, &non_html).await;
    seed_descriptor(
        pool,
        non_html.release_id,
        "plain",
        "static",
        "plain",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'plain', 'index.html', $2, 'file', 'text/plain')",
    )
    .bind(non_html.release_id)
    .bind(plain_artifact)
    .execute(pool)
    .await
    .expect("seed non-HTML static file");
    publish(pool, non_html.release_id, owner).await;

    let overflow = seed_release(pool, owner, repository, 5).await;
    let overflow_agent = seed_agent(pool, repository, overflow.release_id, 5).await;
    seed_ui_source(pool, &overflow).await;
    for index in 0..17 {
        let key = format!("overflow-{index}");
        seed_descriptor(
            pool,
            overflow.release_id,
            &key,
            "managed_service",
            &format!("route-{index}"),
            "index.html",
        )
        .await;
        sqlx::query(
            "INSERT INTO release_ui_managed_services
             (release_id, ui_key, gateway_name, route, release_agent_id)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(overflow.release_id)
        .bind(&key)
        .bind(format!("gateway-{index}"))
        .bind(format!("/service-{index}"))
        .bind(overflow_agent)
        .execute(pool)
        .await
        .expect("seed overflow managed UI service");
    }
    publish(pool, overflow.release_id, owner).await;

    let file_overflow = seed_release(pool, owner, repository, 6).await;
    let overflow_artifact = seed_artifact(pool, file_overflow.release_id, 6, "text/html").await;
    seed_ui_source(pool, &file_overflow).await;
    seed_descriptor(
        pool,
        file_overflow.release_id,
        "files",
        "static",
        "files",
        "index.html",
    )
    .await;
    for index in 0..257 {
        let route = if index == 0 {
            String::from("index.html")
        } else {
            format!("file-{index}.js")
        };
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'files', $2, $3, 'file', 'text/html')",
        )
        .bind(file_overflow.release_id)
        .bind(route)
        .bind(overflow_artifact)
        .execute(pool)
        .await
        .expect("seed per-UI static file overflow");
    }
    publish(pool, file_overflow.release_id, owner).await;

    let api_overflow = seed_release(pool, owner, repository, 7).await;
    let api_agent = seed_agent(pool, repository, api_overflow.release_id, 7).await;
    seed_ui_source(pool, &api_overflow).await;
    seed_descriptor(
        pool,
        api_overflow.release_id,
        "apis",
        "managed_service",
        "apis",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'apis', 'gateway', '/service', $2)",
    )
    .bind(api_overflow.release_id)
    .bind(api_agent)
    .execute(pool)
    .await
    .expect("seed API overflow managed service");
    for index in 0..17 {
        sqlx::query(
            "INSERT INTO release_ui_api_bindings
             (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
             VALUES ($1, 'apis', $2, 'gateway', 'GET', $3, $4)",
        )
        .bind(api_overflow.release_id)
        .bind(format!("api-{index}"))
        .bind(format!("/api-{index}"))
        .bind(api_agent)
        .execute(pool)
        .await
        .expect("seed per-UI API overflow");
    }
    publish(pool, api_overflow.release_id, owner).await;

    Seeded {
        owner,
        outsider,
        valid,
        legacy,
        missing_managed,
        non_html,
        overflow,
        file_overflow,
        api_overflow,
        html_artifact,
        agent_id,
    }
}
