//! Explicit operator resolution through the authenticated forge Git boundary.

use hephaestus_app::RunningHephaestus;
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Replays Bob's reviewed recipe onto Alice's approved canonical head without
/// changing the conflicted proposal or its original frozen provenance.
pub async fn resolve(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    repository_id: Uuid,
    bob_run_id: Uuid,
    approved_head: &str,
) -> String {
    let original: (String, String, String, String) = sqlx::query_as(
        "SELECT result.result_ref, result.result_commit, proposal.input_commit, proposal.state
         FROM run_results result JOIN review_proposals proposal ON proposal.run_id = result.run_id
         WHERE result.run_id = $1",
    )
    .bind(bob_run_id)
    .fetch_one(pool)
    .await
    .expect("conflicted recipe provenance");
    assert_eq!(original.3, "conflicted");
    let (checkout, token, resolved_head) =
        prepare_resolution_checkout(root, running, repository_id, approved_head, &original).await;
    super::authenticated_git(
        &checkout,
        &token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await;
    let bare = root
        .join("repositories")
        .join(format!("{repository_id}.git"));
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        resolved_head
    );
    assert_canonical_recipes(&bare, &resolved_head).await;
    assert_retained_history(
        pool,
        repository_id,
        bob_run_id,
        approved_head,
        &resolved_head,
        &original,
    )
    .await;
    resolved_head
}

async fn prepare_resolution_checkout(
    root: &Path,
    running: &RunningHephaestus,
    repository_id: Uuid,
    approved_head: &str,
    original: &(String, String, String, String),
) -> (PathBuf, String, String) {
    let checkout = root.join("operator-conflict-resolution");
    let remote = format!("http://{}/{repository_id}", running.http_addr());
    let token = super::signed_token();
    super::authenticated_git(
        root,
        &token,
        &["clone", &remote, checkout.to_str().expect("checkout path")],
    )
    .await;
    super::git(&checkout, &["config", "user.name", "Cooking operator"]).await;
    super::git(
        &checkout,
        &["config", "user.email", "golden@example.invalid"],
    )
    .await;
    assert_eq!(
        super::git_output(&checkout, &["rev-parse", "HEAD"]).await,
        approved_head
    );
    super::authenticated_git(&checkout, &token, &["fetch", "origin", &original.0]).await;
    assert_eq!(
        super::git_output(&checkout, &["rev-parse", "FETCH_HEAD"]).await,
        original.1
    );
    super::git(&checkout, &["cherry-pick", &original.1]).await;
    let resolved_head = super::git_output(&checkout, &["rev-parse", "HEAD"]).await;
    assert_ne!(resolved_head, original.1);
    assert_eq!(
        super::git_output(&checkout, &["rev-parse", "HEAD^"]).await,
        approved_head,
        "operator resolution preserves the approved recipe as its parent"
    );
    assert_eq!(
        super::git_output(&checkout, &["diff", "--name-only", approved_head, "HEAD"]).await,
        "content/recipes/recipe-43.md",
        "resolution imports only the inspected competing recipe"
    );
    (checkout, token, resolved_head)
}

async fn assert_canonical_recipes(bare: &Path, resolved_head: &str) {
    for (recipe, title) in [(42, "Family pasta"), (43, "Family soup")] {
        let content = super::git_output_bare(
            bare,
            &[
                "show",
                &format!("{resolved_head}:content/recipes/recipe-{recipe}.md"),
            ],
        )
        .await;
        assert!(
            content.contains(title),
            "canonical recipe {recipe} survives resolution"
        );
    }
}

async fn assert_retained_history(
    pool: &PgPool,
    repository_id: Uuid,
    bob_run_id: Uuid,
    approved_head: &str,
    resolved_head: &str,
    original: &(String, String, String, String),
) {
    let retained: (String, String, String) = sqlx::query_as(
        "SELECT result.result_commit, proposal.input_commit, proposal.state
         FROM run_results result JOIN review_proposals proposal ON proposal.run_id = result.run_id
         WHERE result.run_id = $1",
    )
    .bind(bob_run_id)
    .fetch_one(pool)
    .await
    .expect("retained conflict history");
    assert_eq!(
        retained,
        (original.1.clone(), original.2.clone(), original.3.clone())
    );
    let accepted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives receive
         JOIN git_ref_updates change ON change.receive_id = receive.id
         JOIN external_identities actor ON actor.user_id = receive.actor_id
         WHERE receive.repository_id = $1 AND receive.status = 'accepted'
           AND change.git_ref = 'refs/heads/main' AND change.old_commit = $2
           AND change.new_commit = $3 AND actor.issuer = $4 AND actor.subject = 'golden-subject'",
    )
    .bind(repository_id)
    .bind(approved_head)
    .bind(resolved_head)
    .bind(super::golden_issuer())
    .fetch_one(pool)
    .await
    .expect("authorized operator Git receive");
    assert_eq!(
        accepted, 1,
        "resolution crosses the authenticated forge boundary once"
    );
}
