// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(crate) async fn exercise_follow_up(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &SeededInstance,
    gateway: &GatewayGoldenFixture,
    root: &Path,
    repository: uuid::Uuid,
    input_commit: &str,
    checkpoint: CookingCheckpoint,
    _upstream: &BrokeredTlsUpstream,
    owner_browser_session: BrowserSessionSid,
    outsider: UserId,
    outsider_browser_session: BrowserSessionSid,
) -> String {
    let CookingCheckpoint { alice, bob } = checkpoint;
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = caddy_gateway_client();
    // This request is deliberately sent only after the supervisor restart;
    // its model request must carry Alice's persisted SQLite summary.
    let follow_up = deliver_request(pool, gateway, &client, &url, 44, 1001, "salad").await;
    assert_ne!(follow_up.event_id, alice.event_id);
    assert_normalized_mailbox_body(pool, follow_up.event_id, 44, "alice", "salad").await;
    // The response-loss helper already records one durable replay of update
    // 42; the explicit duplicate in exercise_initial records the second.
    assert_eq!(mailbox_counts(pool, gateway).await, (3, 2, 3));
    let cooking_run_count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT attempt.run_id)
           FROM mailbox_delivery_attempts attempt
           JOIN mailbox_events event ON event.id = attempt.event_id
          WHERE event.mailbox_id = $1",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("cooking run count");
    if cooking_run_count != 3 {
        // Keep the exact response-loss event's attempt lineage available even
        // when the assertion aborts the test before the next periodic sample.
        write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), Some(alice.event_id))
            .await;
    }
    assert_eq!(
        cooking_run_count, 3,
        "duplicate ingress must not start a run"
    );
    let leases: Vec<(time::OffsetDateTime, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT lease.acquired_at, lease.released_at
           FROM agent_instance_volume_leases lease
          WHERE lease.run_id = ANY($1)
          ORDER BY lease.acquired_at",
    )
    .bind(vec![
        alice.run_id.as_uuid(),
        bob.run_id.as_uuid(),
        follow_up.run_id.as_uuid(),
    ])
    .fetch_all(pool)
    .await
    .expect("cooking lease windows");
    assert_eq!(leases.len(), 3, "each logical recipe gets one state lease");
    for pair in leases.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "state leases for cooking runs must not overlap"
        );
    }

    let run_id = alice.run_id;
    running
        .wait_for_run_event(
            run_id,
            hephaestus_app::RunEventKind::ResultCompleted,
            Duration::from_secs(30),
        )
        .await
        .expect("controlled cooking result");
    let (result_ref, result_commit): (String, String) = sqlx::query_as(
        "SELECT result_ref,result_commit FROM run_results WHERE run_id=$1 AND state='completed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("controlled result provenance");
    let bare = root.join("repositories").join(format!("{repository}.git"));
    assert_eq!(
        crate::git_output_bare(&bare, &["rev-parse", &format!("{result_commit}^")]).await,
        input_commit
    );
    let recipe = crate::git_output_bare(
        &bare,
        &[
            "show",
            &format!("{result_commit}:content/recipes/recipe-42.md"),
        ],
    )
    .await;
    assert!(recipe.contains("Family pasta"));
    for (run, recipe_name, expected_title) in [
        (bob.run_id, "recipe-43.md", "Family soup"),
        (follow_up.run_id, "recipe-44.md", "Family salad"),
    ] {
        running
            .wait_for_run_event(
                run,
                hephaestus_app::RunEventKind::ResultCompleted,
                Duration::from_secs(30),
            )
            .await
            .expect("controlled cooking result");
        let commit: String = sqlx::query_scalar(
            "SELECT result_commit FROM run_results
              WHERE run_id = $1 AND state = 'completed'",
        )
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("controlled result commit");
        assert_eq!(
            crate::git_output_bare(&bare, &["rev-parse", &format!("{commit}^")]).await,
            input_commit,
            "competing cooking result retains its frozen Git target"
        );
        let page = crate::git_output_bare(
            &bare,
            &["show", &format!("{commit}:content/recipes/{recipe_name}")],
        )
        .await;
        assert!(page.contains(expected_title));
    }
    let bob_view = super::super::cooking_inspection::inspect(
        pool,
        running,
        bob.run_id.as_uuid(),
        bob.event_id,
        false,
        owner_browser_session,
        outsider.as_uuid(),
        outsider_browser_session,
    )
    .await;
    super::super::cooking_inspection::inspect(
        pool,
        running,
        follow_up.run_id.as_uuid(),
        follow_up.event_id,
        false,
        owner_browser_session,
        outsider.as_uuid(),
        outsider_browser_session,
    )
    .await;
    assert_eq!(
        crate::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        input_commit,
        "canonical Git remains controlled"
    );
    let evidence:(uuid::Uuid,uuid::Uuid,i64)=sqlx::query_as("SELECT run.instance_revision_id,lease.id,delivery.dispatch_sequence FROM runs run JOIN agent_instance_volume_leases lease ON lease.run_id=run.id JOIN mailbox_delivery_attempts attempt ON attempt.run_id=run.id JOIN mailbox_deliveries delivery ON delivery.event_id=attempt.event_id WHERE run.id=$1").bind(run_id.as_uuid()).fetch_one(pool).await.expect("revision/state lease/dispatch provenance");
    assert_ne!(evidence.0, instance.revision, "run uses bound revision");
    assert!(evidence.2 > 0);
    super::super::cooking_inspection::inspect(
        pool,
        running,
        run_id.as_uuid(),
        alice.event_id,
        true,
        owner_browser_session,
        outsider.as_uuid(),
        outsider_browser_session,
    )
    .await;
    assert_eq!(
        crate::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        result_commit,
        "authorized host approval publishes the exact recipe result"
    );
    // Bob's proposal was created against the frozen input commit. Once Alice
    // advances canonical main, approving Bob must settle as a Git conflict and
    // retain the competing proposal history without moving the canonical ref.
    super::super::cooking_inspection::approve_for_test(
        pool,
        running,
        &bob_view,
        owner_browser_session,
    )
    .await;
    let bob_proposal: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM review_proposals WHERE run_id = $1")
            .bind(bob.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("Bob competing proposal");
    let bob_proposal_state: String =
        sqlx::query_scalar("SELECT state FROM review_proposals WHERE id = $1")
            .bind(bob_proposal)
            .fetch_one(pool)
            .await
            .expect("Bob proposal disposition");
    assert_eq!(bob_proposal_state, "conflicted");
    let conflict_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events
          WHERE run_id = $1 AND event_type = 'review.conflicted'",
    )
    .bind(bob.run_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("Bob conflict history");
    assert_eq!(conflict_events, 1, "stale approval remains durable history");
    assert_eq!(
        crate::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        result_commit,
        "stale competing approval cannot move canonical Git"
    );
    let resolved_head = super::super::cooking_conflicts::resolve(
        pool,
        running,
        root,
        repository,
        bob.run_id.as_uuid(),
        &result_commit,
    )
    .await;
    assert_ne!(resolved_head, result_commit);

    // These two requests exercise retryable provider faults after the daemon
    // restart. Each has one durable logical recipe and two physical attempts;
    // the relay fault commits before dropping its response, so its retry must
    // resolve the existing deterministic ledger row.
    let model_fault = deliver_request(
        pool,
        gateway,
        &client,
        &url,
        MODEL_FAULT_UPDATE,
        1001,
        "pancakes",
    )
    .await;
    assert_retried_delivery(pool, gateway.mailbox_id.as_uuid(), model_fault).await;
    super::super::cooking_inspection::inspect_with_https_uses(
        pool,
        running,
        model_fault.run_id.as_uuid(),
        model_fault.event_id,
        false,
        4,
        owner_browser_session,
        outsider.as_uuid(),
        outsider_browser_session,
    )
    .await;
    let relay_fault = deliver_request(
        pool,
        gateway,
        &client,
        &url,
        RELAY_FAULT_UPDATE,
        1002,
        "waffles",
    )
    .await;
    assert_retried_delivery(pool, gateway.mailbox_id.as_uuid(), relay_fault).await;
    super::super::cooking_inspection::inspect_with_https_uses(
        pool,
        running,
        relay_fault.run_id.as_uuid(),
        relay_fault.event_id,
        false,
        2,
        owner_browser_session,
        outsider.as_uuid(),
        outsider_browser_session,
    )
    .await;
    // Update 42 has two deliberate duplicate publications: the response-loss
    // helper's direct replay and the explicit replay in exercise_initial.
    // The two fault requests add accepted publications without duplicates.
    assert_eq!(mailbox_counts(pool, gateway).await, (5, 2, 5));
    if let Ok(hugo) = env::var("HEPHAESTUS_COOKING_HUGO") {
        let checkout = root.join("cooking-site-build");
        crate::git(
            root,
            &[
                "clone",
                bare.to_str().expect("bare path"),
                checkout.to_str().expect("build path"),
            ],
        )
        .await;
        let output = tokio::process::Command::new(hugo)
            .arg("--source")
            .arg(&checkout)
            .output()
            .await
            .expect("pinned Hugo build");
        assert!(
            output.status.success(),
            "generated recipe passes Hugo build"
        );
        assert!(
            checkout
                .join("public/recipes/recipe-42/index.html")
                .is_file()
        );
    }
    eprintln!(
        "cooking journey: mailbox={}, runs=[{},{},{}], revision={}, lease={}, result_ref={}, result_commit={}",
        gateway.mailbox_id,
        alice.run_id,
        bob.run_id,
        follow_up.run_id,
        evidence.0,
        evidence.1,
        result_ref,
        result_commit
    );
    write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
    resolved_head
}

pub(crate) type CookingLineageRow = (
    uuid::Uuid,
    uuid::Uuid,
    i32,
    uuid::Uuid,
    String,
    time::OffsetDateTime,
    Option<time::OffsetDateTime>,
    String,
    Option<String>,
    Option<i32>,
    Option<i32>,
    String,
    Option<time::OffsetDateTime>,
    Option<time::OffsetDateTime>,
    time::OffsetDateTime,
    time::OffsetDateTime,
);
