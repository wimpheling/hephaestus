use super::{
    UiManifestEntryKind, UiManifestInspection, UiManifestStatus, persist_ui_manifest_revision,
};
use forge_domain::{CommitSha, ReceiveId, RepositoryId};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, time::Duration};
use uuid::Uuid;

const STATIC_UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[uis.content]
kind = "static"
entrypoint = "index.html"

[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn persists_reuses_and_rejects_conflicting_ui_capture_evidence() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("SKIPPED UI manifest store: HEPHAESTUS_POSTGRES_TEST_URL is unset");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::migrate!("../../../../migrations")
        .run(&admin)
        .await
        .expect("apply migrations");
    let worker = worker_pool(&database_url).await;
    let fixture = seed_fixture(&admin).await;

    let valid = valid_inspection();
    let commit = CommitSha::parse(repeated_hex('a')).expect("commit");
    let first = {
        let mut transaction = worker.begin().await.expect("begin first transaction");
        let result = persist_ui_manifest_revision(
            &mut transaction,
            fixture.repository,
            fixture.first_receive,
            &commit,
            &valid,
        )
        .await
        .expect("insert valid UI capture");
        transaction.commit().await.expect("commit first capture");
        result
    };
    assert_eq!(first.status, UiManifestStatus::Valid);
    assert!(first.normalized_ui_hash.is_some());
    assert!(first.normalized_gateway_hash.is_none());

    let reused = {
        let mut transaction = worker.begin().await.expect("begin reuse transaction");
        let result = persist_ui_manifest_revision(
            &mut transaction,
            fixture.repository,
            fixture.second_receive,
            &commit,
            &valid,
        )
        .await
        .expect("reuse identical UI capture");
        transaction.commit().await.expect("commit reuse");
        result
    };
    assert_eq!(reused, first, "later receives reuse the immutable revision");

    let stored_receive: Uuid =
        sqlx::query_scalar("SELECT receive_id FROM ui_source_manifest_revisions WHERE id = $1")
            .bind(first.id)
            .fetch_one(&worker)
            .await
            .expect("original capture provenance");
    assert_eq!(stored_receive, fixture.first_receive.as_uuid());

    let mut conflicting = valid_inspection();
    conflicting.object_id = object_id('c');
    let mut transaction = worker.begin().await.expect("begin conflict transaction");
    let error = persist_ui_manifest_revision(
        &mut transaction,
        fixture.repository,
        fixture.third_receive,
        &commit,
        &conflicting,
    )
    .await
    .expect_err("conflicting source evidence must fail closed");
    assert_eq!(
        error.to_string(),
        "invalid forge metadata: conflicting repository UI capture evidence"
    );
    transaction
        .rollback()
        .await
        .expect("rollback conflicting capture");

    let row_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions WHERE repository_id = $1",
    )
    .bind(fixture.repository.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("capture count");
    assert_eq!(row_count, 1, "conflicting transaction left no partial row");

    let invalid_commit = CommitSha::parse(repeated_hex('b')).expect("invalid commit");
    let invalid = invalid_inspection();
    let invalid_result = {
        let mut transaction = worker.begin().await.expect("begin invalid transaction");
        let result = persist_ui_manifest_revision(
            &mut transaction,
            fixture.repository,
            fixture.third_receive,
            &invalid_commit,
            &invalid,
        )
        .await
        .expect("persist invalid UI capture");
        transaction.commit().await.expect("commit invalid capture");
        result
    };
    assert_eq!(invalid_result.status, UiManifestStatus::Invalid);
    assert!(invalid_result.normalized_ui_hash.is_none());

    let invalid_row_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND status = 'invalid'",
    )
    .bind(fixture.repository.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("invalid capture count");
    assert_eq!(invalid_row_count, 1);

    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE repository_id = $1",
    )
    .bind(fixture.repository.as_uuid())
    .fetch_one(&worker)
    .await
    .expect("shared snapshot links");
    assert_eq!(links, 0, "capture persistence creates no build links");
}

#[test]
fn rejects_sizes_that_do_not_fit_postgres_bigint() {
    let mut inspection = invalid_inspection();
    inspection.actual_size = Some(i64::MAX as u64 + 1);
    let error = super::evidence(&inspection).expect_err("oversized source length");
    assert_eq!(
        error.to_string(),
        "invalid stored forge data in UI manifest size"
    );
}

fn valid_inspection() -> UiManifestInspection {
    let parsed = agent_config::parse_repository_uis(STATIC_UI.as_bytes());
    UiManifestInspection {
        status: UiManifestStatus::Valid,
        requires_gateways: false,
        entry_kind: UiManifestEntryKind::Regular,
        object_id: object_id('a'),
        actual_size: Some(STATIC_UI.len() as u64),
        source_hash: Some(parsed.hash),
        normalized_hash: parsed.normalized_hash,
        config: parsed.config,
        gateway_object_id: None,
        gateway_actual_size: None,
        gateway_source_hash: None,
        gateway_normalized_hash: None,
        gateway_config: None,
        diagnostics: Vec::new(),
    }
}

fn invalid_inspection() -> UiManifestInspection {
    UiManifestInspection {
        status: UiManifestStatus::Invalid,
        requires_gateways: false,
        entry_kind: UiManifestEntryKind::Regular,
        object_id: object_id('b'),
        actual_size: Some(32),
        source_hash: None,
        normalized_hash: None,
        config: None,
        gateway_object_id: None,
        gateway_actual_size: None,
        gateway_source_hash: None,
        gateway_normalized_hash: None,
        gateway_config: None,
        diagnostics: vec![agent_config::Diagnostic {
            code: String::from("invalid_repository_ui_manifest"),
            path: Some(String::from("manifest")),
            message: String::from("repository manifest is invalid"),
        }],
    }
}

fn object_id(first: char) -> gix::ObjectId {
    gix::ObjectId::from_hex(repeated_hex(first).as_bytes()).expect("object ID")
}

fn repeated_hex(first: char) -> String {
    format!("{first}{}", "0".repeat(39))
}

struct Fixture {
    repository: RepositoryId,
    first_receive: ReceiveId,
    second_receive: ReceiveId,
    third_receive: ReceiveId,
}

async fn seed_fixture(pool: &PgPool) -> Fixture {
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = RepositoryId::new();
    let first_receive = ReceiveId::new();
    let second_receive = ReceiveId::new();
    let third_receive = ReceiveId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("ui-store-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("ui-store-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository.as_uuid())
        .bind(project)
        .bind(format!("ui-store-{repository}"))
        .execute(pool)
        .await
        .expect("repository");
    for receive in [first_receive, second_receive, third_receive] {
        sqlx::query(
            "INSERT INTO git_receives (id, repository_id, principal, status)
             VALUES ($1, $2, 'ui-store-test', 'accepted')",
        )
        .bind(receive.as_uuid())
        .bind(repository.as_uuid())
        .execute(pool)
        .await
        .expect("receive");
    }
    Fixture {
        repository,
        first_receive,
        second_receive,
        third_receive,
    }
}

async fn worker_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(&mut *connection)
                    .await?;
                Ok::<(), sqlx::Error>(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect worker role")
}
