use uuid::Uuid;

const TARGET_PUBLICATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(super) async fn product_event_id(pool: &sqlx::PgPool, request: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM application_events
           WHERE request_id = $1 AND aggregate_type = 'organization'
           ORDER BY cursor DESC LIMIT 1",
    )
    .bind(request)
    .fetch_one(pool)
    .await
    .expect("product event id")
}

pub(super) async fn unpublished_scope_event_ids(pool: &sqlx::PgPool, scope_id: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT event.id FROM product_event_outbox outbox
           JOIN application_events event ON event.id = outbox.event_id
          WHERE event.scope_kind = 'organization' AND event.scope_id = $1
            AND outbox.published_at IS NULL
            AND outbox.dead_lettered_at IS NULL
          ORDER BY event.cursor",
    )
    .bind(scope_id)
    .fetch_all(pool)
    .await
    .expect("unpublished scope event ids")
}

pub(super) async fn publish_events_until_published(
    pool: &sqlx::PgPool,
    publisher: &crate::event_adapter::EventPublisher,
    event_ids: &[Uuid],
) {
    assert!(!event_ids.is_empty(), "publication target is nonempty");
    tokio::time::timeout(TARGET_PUBLICATION_TIMEOUT, async {
        loop {
            let (found, delivered_count, dead_lettered): (i64, i64, i64) = sqlx::query_as(
                "SELECT count(*),
                        count(*) FILTER (WHERE published_at IS NOT NULL),
                        count(*) FILTER (WHERE dead_lettered_at IS NOT NULL)
                   FROM product_event_outbox
                  WHERE event_id = ANY($1)",
            )
            .bind(event_ids.to_vec())
            .fetch_one(pool)
            .await
            .expect("publication target status");
            assert_eq!(
                found,
                i64::try_from(event_ids.len()).expect("event target count fits in i64"),
                "all publication targets have outbox rows"
            );
            assert_eq!(dead_lettered, 0, "publication targets were dead-lettered");
            if delivered_count == found {
                return;
            }
            // The CI database is shared by the workspace tests. The real
            // outbox adapter orders every pending event globally, so a fixed
            // batch can repeatedly miss this fixture's events behind older
            // rows. Drain the current pending set through the real publisher.
            let pending_count: i64 = sqlx::query_scalar(
                "SELECT count(*)
                   FROM product_event_outbox
                  WHERE published_at IS NULL AND dead_lettered_at IS NULL",
            )
            .fetch_one(pool)
            .await
            .expect("pending product event count");
            publisher
                .publish_pending(pending_count.max(1))
                .await
                .expect("publish targeted events");
        }
    })
    .await
    .expect("targeted product events published before bounded timeout");
}

pub(super) async fn mutate_organization(
    pool: &sqlx::PgPool,
    actor: Uuid,
    organization: Uuid,
    suffix: &str,
) -> Uuid {
    let mut transaction = pool.begin().await.expect("organization transaction");
    let request = Uuid::new_v4();
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true),
                  set_config('hephaestus.occurrence_id', $2, true)",
    )
    .bind(actor.to_string())
    .bind(request.to_string())
    .execute(&mut *transaction)
    .await
    .expect("organization actor");
    sqlx::query("UPDATE organizations SET name = $2 WHERE id = $1")
        .bind(organization)
        .bind(format!("watch-{suffix}-{organization}"))
        .execute(&mut *transaction)
        .await
        .expect("mutate organization");
    transaction.commit().await.expect("commit organization");
    request
}
