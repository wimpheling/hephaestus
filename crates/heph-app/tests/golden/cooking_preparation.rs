use super::*;

pub async fn join_cooking_preparation<A, B>(first: A, second: B) -> (A::Output, B::Output)
where
    A: std::future::Future,
    B: std::future::Future,
{
    use futures_util::FutureExt as _;

    let (first, second) = tokio::join!(
        std::panic::AssertUnwindSafe(first).catch_unwind(),
        std::panic::AssertUnwindSafe(second).catch_unwind(),
    );
    (
        first.unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        second.unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
    )
}

#[tokio::test]
pub async fn cooking_preparation_overlaps_child_lifecycles() {
    let root = tempfile::tempdir().expect("preparation lifecycle root");
    let child = |own: &'static str, peer: &'static str| {
        let root = root.path();
        async move {
            let status = Command::new("sh")
                .args([
                    "-eu", "-c",
                    r#"touch "$1/$2"; while [ ! -f "$1/$3" ]; do sleep 0.01; done; touch "$1/$2-finished""#,
                    "cooking-preparation", root.to_str().expect("fixture path"), own, peer,
                ])
                .kill_on_drop(true)
                .status()
                .await
                .expect("preparation lifecycle child");
            assert!(status.success());
        }
    };
    // Each real child waits for the other to start. Serial polling deadlocks
    // and hits this bounded fixture timeout instead of passing spuriously.
    tokio::time::timeout(
        Duration::from_secs(5),
        join_cooking_preparation(child("release", "blog"), child("blog", "release")),
    )
    .await
    .expect("both preparation children must make progress");
    assert!(root.path().join("release-finished").is_file());
    assert!(root.path().join("blog-finished").is_file());
}

#[tokio::test]
pub async fn cooking_preparation_drains_child_before_resuming_either_panic() {
    use futures_util::FutureExt as _;

    for first_panics in [true, false] {
        let root = tempfile::tempdir().expect("preparation failure root");
        let branch = |panics: bool| {
            let root = root.path();
            async move {
                if panics {
                    // Preserve a real unwind payload without printing an expected
                    // fixture panic into the full Cooking workload diagnostics.
                    std::panic::resume_unwind(Box::new("original preparation failure"));
                }
                let status = Command::new("sh")
                    .args([
                        "-eu",
                        "-c",
                        r#"sleep 0.02; touch "$1/finished""#,
                        "cooking-preparation",
                        root.to_str().expect("fixture path"),
                    ])
                    .kill_on_drop(true)
                    .status()
                    .await
                    .expect("preparation cleanup child");
                assert!(status.success());
            }
        };
        let failure = tokio::time::timeout(
            Duration::from_secs(5),
            std::panic::AssertUnwindSafe(join_cooking_preparation(
                branch(first_panics),
                branch(!first_panics),
            ))
            .catch_unwind(),
        )
        .await
        .expect("preparation failure drain deadline")
        .expect_err("original failure must propagate");
        assert_eq!(
            failure.downcast_ref::<&str>(),
            Some(&"original preparation failure"),
        );
        assert!(root.path().join("finished").is_file());
    }
}

#[tokio::test]
#[serial]
pub async fn cooking_confinement_scan_matches_fresh_migrated_schema() {
    let Ok(parent_database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping confinement schema coverage: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let isolated = IsolatedGoldenDatabase::create(&parent_database_url).await;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&isolated.target_url)
        .await
        .expect("connect fresh migrated confinement database");
    cooking_confinement::assert_static_storage_scan_matches_catalog(&pool).await;
    isolated.cleanup(pool).await;
}
