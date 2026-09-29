mod authorize;
mod bind;
mod create;
mod finalize;
mod pinned;
mod raw_update;
mod resolve;
mod rotate;
mod runtime;

#[tokio::test]
#[serial_test::serial]
async fn encrypted_rotation_delegation_revocation_and_purge_are_atomic() {
    let Some(mut state) = create::initialize().await else {
        return;
    };
    create::create_secrets(&mut state).await;
    authorize::authorize_and_bind_setup(&mut state).await;
    bind::bind_and_validate(&mut state).await;
    resolve::resolve_and_publish(&mut state).await;
    runtime::exercise_brokered_runtime(&state).await;
    raw_update::prepare_raw_and_update(&mut state).await;
    pinned::prepare_pinned_leases(&mut state).await;
    rotate::rotate_and_revoke(&mut state).await;
    finalize::finalize_and_purge(&state).await;
}
