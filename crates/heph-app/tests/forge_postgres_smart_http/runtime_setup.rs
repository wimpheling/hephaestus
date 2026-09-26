use super::*;

pub(super) struct RuntimeFixture {
    pub(super) runtime_session_id: Uuid,
    pub(super) origin_attachment: Uuid,
    pub(super) expired_session_id: Uuid,
    pub(super) expired_credential: RuntimeGitCredential,
    pub(super) revoked_session_id: Uuid,
    pub(super) revoked_credential: RuntimeGitCredential,
    pub(super) runtime_authenticator: Arc<RuntimeGitHttpAuthenticator>,
    pub(super) runtime_server: tokio::task::JoinHandle<()>,
    pub(super) runtime_remote: String,
    pub(super) other_runtime_remote: String,
    pub(super) runtime_credential_header: String,
}

pub(super) async fn prepare_runtime(ctx: &SmartHttpContext) -> RuntimeFixture {
    let (runtime_session_id, origin_attachment, runtime_credential, runtime_binding_id) =
        seed_runtime_transport_authority(
            &ctx.pool,
            ctx.repository.id.as_uuid(),
            &ctx.runtime_commit,
            ctx.user_id,
            ctx.temporary.path().join("runtime-git-handoff"),
            None,
            time::Duration::minutes(10),
        )
        .await;
    let (expired_session_id, _, expired_credential, _) = seed_runtime_transport_authority(
        &ctx.pool,
        ctx.repository.id.as_uuid(),
        &ctx.runtime_commit,
        ctx.user_id,
        ctx.temporary.path().join("expired-runtime-git-handoff"),
        Some(runtime_binding_id),
        time::Duration::seconds(1),
    )
    .await;
    let (revoked_session_id, _, revoked_credential, _) = seed_runtime_transport_authority(
        &ctx.pool,
        ctx.repository.id.as_uuid(),
        &ctx.runtime_commit,
        ctx.user_id,
        ctx.temporary.path().join("revoked-runtime-git-handoff"),
        Some(runtime_binding_id),
        time::Duration::minutes(10),
    )
    .await;
    // Prerequisite: `cargo build -p git-http --bin pre-receive`, unless
    // HEPHAESTUS_GIT_RECEIVE_HOOK points to an absolute executable hook.
    let hook = runtime_receive_hook_path();
    let runtime_token = runtime_credential.expose_token().to_string();
    let runtime_credential_header = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-runtime:{runtime_token}"))
    );
    let runtime_authenticator = Arc::new(RuntimeGitHttpAuthenticator::new(Arc::new(
        PgRuntimeGitCredentialRepository::new(ctx.pool.clone()),
    )));
    runtime_authenticator
        .authenticate_git(
            Some(&runtime_credential_header),
            RequestId::new(),
            ctx.repository.id,
            GitOperation::Push,
        )
        .await
        .expect("direct production runtime Git authentication");
    let runtime_router = GitHttpService::new(
        Arc::clone(&ctx.repository_service),
        Arc::clone(&ctx.storage),
        runtime_authenticator.clone(),
        ctx.authorizer.clone(),
        ctx.backend.clone(),
        GitHttpLimits::default(),
    )
    .expect("runtime Git HTTP configuration")
    .with_runtime_receive_hook(hook)
    .expect("runtime receive hook configuration")
    .router();
    let runtime_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("runtime listener");
    let runtime_address = runtime_listener
        .local_addr()
        .expect("runtime listener address");
    let runtime_server = tokio::spawn(async move {
        axum::serve(runtime_listener, runtime_router)
            .await
            .expect("runtime smart HTTP server");
    });
    let runtime_remote = format!("http://{runtime_address}/{}", ctx.repository.id);
    let other_runtime_remote = format!("http://{runtime_address}/{}", ctx.other_repository.id);
    RuntimeFixture {
        runtime_session_id,
        origin_attachment,
        expired_session_id,
        expired_credential,
        revoked_session_id,
        revoked_credential,
        runtime_authenticator,
        runtime_server,
        runtime_remote,
        other_runtime_remote,
        runtime_credential_header,
    }
}
