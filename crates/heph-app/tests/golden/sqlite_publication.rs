//! Focused real Python image, Git build, artifact import and authenticated publication.

#[path = "sqlite_publication/evidence.rs"]
mod evidence;
#[path = "sqlite_publication/probe.rs"]
mod probe;

pub use evidence::build_and_publish;

use crate::*;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[ignore = "requires dedicated real libkrun/Python image/PostgreSQL/NATS fixture"]
async fn configured_source_build_and_authenticated_publication() {
    assert_eq!(
        env::var("HEPHAESTUS_APP_SQLITE_PUBLICATION").as_deref(),
        Ok("1")
    );
    let mode = prepare_golden_run_mode()
        .await
        .expect("explicit native services");
    assert!(mode.libkrun_e2e && !mode.release_build_proof);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&mode.database_url)
        .await
        .expect("fresh published database");
    let seed = seed_golden_scenario(
        &pool, false, true, true, false, false, false, false, false, false,
    )
    .await;
    let mut backend = configure_golden_backend(
        &pool,
        &seed.root,
        &seed.repository_root,
        false,
        false,
        true,
        false,
        &mode.database_url,
        &mode.nats_url,
        &seed.browser_oidc_issuer,
        None,
        &None,
        &None,
        Duration::from_secs(90),
    )
    .await;
    let reference = env::var("HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE").expect("immutable Python image");
    assert!(
        reference
            .ends_with("@sha256:24b78e523e8cf1732243dc8945d5c75145b5e21d23081a90ecc6b591d4bb820e")
    );
    probe::verify(&backend.app_config, &backend.root_image).await;
    evidence::load_image(&pool, &reference).await;
    backend.app_config.root_images.insert(
        reference,
        RootFilesystem::Directory {
            host_path: backend.root_image.clone(),
        },
    );
    let running = HephaestusApp::build(backend.app_config)
        .await
        .expect("production build composition")
        .start()
        .await
        .expect("ready real daemon");
    let identity = AuthenticatedIdentity::new(
        seed.user_id,
        &seed.browser_oidc_issuer,
        "golden-subject",
        serde_json::json!({}),
        RequestId::new(),
    );
    let rpc_token =
        |audience: &str| evidence::token(seed.user_id, &seed.owner_browser_session, audience);
    let git_token = signed_token(Duration::from_secs(300));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/local-sqlite")
        .canonicalize()
        .expect("exact example source");
    let context = cooking_builds::CookingBuildContext {
        pool: &pool,
        running: &running,
        root: &seed.root,
        source_root: &source,
        project_id: seed.project.id,
        repositories: &seed.fixture_repository,
        identity: cooking_builds::CookingIdentity {
            actor: &identity,
            git_token: &git_token,
            rpc_token: &rpc_token,
        },
        timeout: Duration::from_secs(120),
    };
    let published = build_and_publish(&context, &source, &seed.release_artifact_root).await;
    running
        .shutdown()
        .await
        .expect("complete real daemon shutdown");
    probe::assert_clean(&format!("build-{}", published.build_request_id));
    cleanup_streams(&mode.nats_url).await;
    finish_isolated_golden(
        mode.isolated_database,
        pool,
        &mode.database_url,
        &mode.parent_database_url,
        mode.parent_before,
    )
    .await;
}
