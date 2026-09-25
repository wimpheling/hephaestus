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
    ActivateUiRequest, DisableUiRequest, ListUiInstallationsRequest, RemoveUiRequest,
    RollbackUiRequest,
};
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) async fn run(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
    state: &InstallScenarioState,
) {
    let installed = &state.installed;
    let target = state.target.clone();
    let list_token = state.list_token.clone();
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

    let rolled_back = release
        .rollback_ui_with_options(
            RollbackUiRequest {
                context: request_context("ui-rollback").into(),
                installation_id: installed.installation_id.clone(),
                expected_generation_id: activated.generation_id.clone(),
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
    assert_ne!(rolled_back.generation_id, activated.generation_id);

    let disabled = release
        .disable_ui_with_options(
            DisableUiRequest {
                context: request_context("ui-disable").into(),
                installation_id: installed.installation_id.clone(),
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

    let disabled_installation_id = disabled
        .installation_id
        .as_option()
        .expect("disabled installation id")
        .value
        .parse::<Uuid>()
        .expect("disabled installation UUID");
    let commands_before_stale: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1",
    )
    .bind(disabled_installation_id)
    .fetch_one(pool)
    .await
    .expect("count lifecycle commands before stale CAS");

    let stale_response = release
        .remove_ui_with_options(
            RemoveUiRequest {
                context: request_context("ui-stale-remove").into(),
                installation_id: disabled.installation_id.clone(),
                expected_generation_id: installed.generation_id.clone(),
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
    .bind(disabled_installation_id)
    .fetch_one(pool)
    .await
    .expect("count lifecycle commands after stale CAS");
    assert_eq!(commands_after_stale, commands_before_stale);
    let removed = release
        .remove_ui_with_options(
            RemoveUiRequest {
                context: request_context("ui-remove").into(),
                installation_id: disabled.installation_id,
                expected_generation_id: disabled.generation_id,
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
    sqlx::query(
        "UPDATE human_browser_sessions
     SET revoked_at = now(), revocation_reason = 'logout'
     WHERE id = $1",
    )
    .bind(fixture.parent_session_id)
    .execute(pool)
    .await
    .expect("revoke parent browser session");
    let revoked_parent = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                organization_id: opaque(fixture.organization_id).into(),
                target: Some(target).into(),
                ..Default::default()
            },
            authorization(&list_token),
        )
        .await
        .expect_err("revoked parent session must be rejected");
    assert_eq!(revoked_parent.code, ErrorCode::Unauthenticated);
}
