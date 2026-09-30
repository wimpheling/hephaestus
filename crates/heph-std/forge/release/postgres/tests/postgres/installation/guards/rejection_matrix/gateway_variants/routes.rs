use super::*;

pub(super) async fn verify(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    public_exposure(context, cases).await;
    paused_gateway(context, cases).await;
    inactive_revision(context, cases).await;
    disabled_route(context, cases).await;
    wrong_method(context, cases).await;
}

async fn public_exposure(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let release_agent_id = context.release_agent_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "public").await;
    let public_revision = seed_gateway_revision_variant(
        admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "public",
        "/service",
        &["GET"],
        true,
        11,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(public_revision)
        .execute(admin_pool)
        .await
        .expect("activate public revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("public-exposure"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore gateway exposure");
}

async fn paused_gateway(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let gateway_id = context.gateway_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "paused").await;
    sqlx::query("UPDATE gateways SET lifecycle = 'paused' WHERE id = $1")
        .bind(gateway_id)
        .execute(admin_pool)
        .await
        .expect("pause gateway");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("paused"),
    )
    .await;
    sqlx::query("UPDATE gateways SET lifecycle = 'enabled' WHERE id = $1")
        .bind(gateway_id)
        .execute(admin_pool)
        .await
        .expect("restore gateway lifecycle");
}

async fn inactive_revision(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "inactive-revision").await;
    sqlx::query("UPDATE gateways SET active_revision_id = NULL WHERE id = $1")
        .bind(gateway_id)
        .execute(admin_pool)
        .await
        .expect("deactivate gateway revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("inactive-revision"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore active gateway revision");
}

async fn disabled_route(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let release_agent_id = context.release_agent_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "disabled-route").await;
    let disabled_route_revision = seed_gateway_revision_variant(
        admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "heph_authenticated",
        "/service",
        &["GET"],
        false,
        12,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(disabled_route_revision)
        .execute(admin_pool)
        .await
        .expect("activate disabled route revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("disabled-route"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore gateway route");
}

async fn wrong_method(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let release_agent_id = context.release_agent_id;
    let gateway_id = context.gateway_id;
    let revision_id = context.revision_id;
    let target = seed_same_org_target_project(admin_pool, fixture, "wrong-method").await;
    let wrong_method_revision = seed_gateway_revision_variant(
        admin_pool,
        revision_id,
        gateway_id,
        release_id,
        release_agent_id,
        "heph_authenticated",
        "/service",
        &["POST"],
        true,
        13,
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(wrong_method_revision)
        .execute(admin_pool)
        .await
        .expect("activate wrong method revision");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("wrong-method"),
    )
    .await;
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(admin_pool)
        .await
        .expect("restore gateway method");
}
