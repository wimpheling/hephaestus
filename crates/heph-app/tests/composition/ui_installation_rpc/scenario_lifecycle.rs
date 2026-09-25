use super::{
    ui_installation_scenario_state::{InstallScenarioState, ReleaseClient},
    ui_installation_seed::Fixture,
    ui_installation_transport::{
        ACTIVATE_AUDIENCE, DISABLE_AUDIENCE, REMOVE_AUDIENCE, ROLLBACK_AUDIENCE, authorization,
        opaque, request_context, session_token,
    },
};
use connectrpc::error::ErrorCode;
use rpc_proto::messages::hephaestus::release::v1::{
    ActivateUiRequest, DisableUiRequest, DisableUiResponse, ListUiInstallationsRequest,
    RemoveUiRequest, RollbackUiRequest, RollbackUiResponse,
};
use sqlx::PgPool;
use uuid::Uuid;

pub(super) async fn run(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
    state: &InstallScenarioState,
) {
    let rolled_back = activate_and_rollback(release, fixture, &state.installed).await;
    let disabled = disable_with_stale_check(pool, release, fixture, &rolled_back).await;
    remove_installation(release, fixture, &disabled).await;
    assert_revoked_parent(pool, release, fixture, state).await;
}

async fn activate_and_rollback(
    release: &ReleaseClient,
    fixture: &Fixture,
    installed: &rpc_proto::messages::hephaestus::release::v1::InstallUiResponse,
) -> RollbackUiResponse {
    let activated = release
        .activate_ui_with_options(
            ActivateUiRequest {
                context: request_context("ui-activate").into(),
                installation_id: installed.installation_id.clone(),
                expected_generation_id: installed.generation_id.clone(),
                release_id: opaque(fixture.release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                ACTIVATE_AUDIENCE,
            )),
        )
        .await
        .expect("activate UI with current CAS")
        .into_owned();
    assert_ne!(activated.generation_id, installed.generation_id);
    let activated_generation_id = activated.generation_id.clone();
    let rolled_back = release
        .rollback_ui_with_options(
            RollbackUiRequest {
                context: request_context("ui-rollback").into(),
                installation_id: installed.installation_id.clone(),
                expected_generation_id: activated_generation_id.clone(),
                release_id: opaque(fixture.release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                ROLLBACK_AUDIENCE,
            )),
        )
        .await
        .expect("rollback UI with current CAS")
        .into_owned();
    assert_ne!(rolled_back.generation_id, activated_generation_id);
    rolled_back
}

async fn disable_with_stale_check(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
    rolled_back: &RollbackUiResponse,
) -> DisableUiResponse {
    let disabled = release
        .disable_ui_with_options(
            DisableUiRequest {
                context: request_context("ui-disable").into(),
                installation_id: rolled_back.installation_id.clone(),
                expected_generation_id: rolled_back.generation_id.clone(),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                DISABLE_AUDIENCE,
            )),
        )
        .await
        .expect("disable UI with current CAS")
        .into_owned();
    assert_eq!(disabled.lifecycle.to_i32(), 2);
    assert_eq!(disabled.generation_id, rolled_back.generation_id);
    let installation_id = disabled
        .installation_id
        .as_option()
        .expect("disabled installation id")
        .value
        .parse::<Uuid>()
        .expect("disabled installation UUID");
    let commands_before_stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(installation_id)
    .fetch_one(pool)
    .await
    .expect("count lifecycle commands before stale CAS");
    let stale_response = release
        .remove_ui_with_options(
            RemoveUiRequest {
                context: request_context("ui-stale-remove").into(),
                installation_id: disabled.installation_id.clone(),
                expected_generation_id: rolled_back.generation_id.clone(),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                REMOVE_AUDIENCE,
            )),
        )
        .await
        .expect_err("removed lifecycle must not accept stale mutation");
    assert_eq!(stale_response.code, ErrorCode::FailedPrecondition);
    let commands_after_stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(installation_id)
    .fetch_one(pool)
    .await
    .expect("count lifecycle commands after stale CAS");
    assert_eq!(commands_after_stale, commands_before_stale);
    disabled
}

async fn remove_installation(
    release: &ReleaseClient,
    fixture: &Fixture,
    disabled: &DisableUiResponse,
) {
    let removed = release
        .remove_ui_with_options(
            RemoveUiRequest {
                context: request_context("ui-remove").into(),
                installation_id: disabled.installation_id.clone(),
                expected_generation_id: disabled.generation_id.clone(),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                REMOVE_AUDIENCE,
            )),
        )
        .await
        .expect("remove UI with current CAS")
        .into_owned();
    assert_eq!(removed.lifecycle.to_i32(), 3);
}

async fn assert_revoked_parent(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
    state: &InstallScenarioState,
) {
    let parent_session_id = fixture.parent_session_id;
    sqlx::query(
        "UPDATE human_browser_sessions
     SET revoked_at = now(), revocation_reason = 'logout'
     WHERE id = $1",
    )
    .bind(parent_session_id)
    .execute(pool)
    .await
    .expect("revoke parent browser session");
    let revoked_parent = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                organization_id: opaque(fixture.organization_id).into(),
                target: Some(state.target.clone()).into(),
                ..Default::default()
            },
            authorization(&state.list_token),
        )
        .await
        .expect_err("revoked parent session must be rejected");
    assert_eq!(revoked_parent.code, ErrorCode::Unauthenticated);
}
