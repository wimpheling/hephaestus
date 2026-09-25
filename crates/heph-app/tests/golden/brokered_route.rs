use super::*;

/// Seeds immutable gateway route and host-only inbound secret authority.
// The fixture deliberately names each persisted authority boundary explicitly.
// Keeping this one setup transaction-shaped makes the golden authority graph
// reviewable without hiding a bound mailbox or grant in a generic helper.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn seed_gateway_brokered_route(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    import_id: uuid::Uuid,
    version_id: uuid::Uuid,
    mailbox_id: MailboxId,
) -> GatewayGoldenFixture {
    let gateway_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let route_id = uuid::Uuid::new_v4();
    let binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateways
           (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'golden-gateway', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
           (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
            release_agent_key, handler_contract, exposure, parameters, secret_slots, mailbox_slots,
            normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'golden-gateway', 'http.v1', 'public',
                 $9, ARRAY['webhook'], $10, $7, $8)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind([8_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        serde_json::json!({"inbound_placeholder": cooking_builds::cooking_inbound_placeholder(version_id), "alice_provider_id":1001, "bob_provider_id":1002})
    } else {
        serde_json::json!({})
    })
    .bind(vec![if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    }])
    .execute(pool)
    .await
    .expect("seed immutable gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
           (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['POST'])",
    )
    .bind(route_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(if cooking::enabled() {
        "/cooking/telegram"
    } else {
        "/brokered"
    })
    .execute(pool)
    .await
    .expect("seed gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate gateway revision");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
           (id, gateway_id, gateway_revision_id, import_id, slot_key,
            secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'webhook', $5, 'active', $6)",
    )
    .bind(binding_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(import_id)
    .bind(version_id)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed gateway secret binding");
    let mailbox_binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
           (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id,
            producer_id, created_by)
         VALUES ($1, $2, $3, $4, $7, $5, 'golden-gateway', $6)",
    )
    .bind(mailbox_binding_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(mailbox_id.as_uuid())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    })
    .execute(pool)
    .await
    .expect("bind gateway fixture mailbox");
    let grant_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
         VALUES ($1, $2, 'active', $3)",
    )
    .bind(grant_id)
    .bind(mailbox_binding_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("grant gateway fixture mailbox publication");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
           (id, binding_id, gateway_revision_id, gateway_route_id, header_name, normalized_hash)
         VALUES ($1, $2, $3, $4, $6, $5)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(binding_id)
    .bind(revision_id)
    .bind(route_id)
    .bind([10_u8; 32].as_slice())
    .bind(if cooking::enabled() {
        "x-telegram-bot-api-secret-token"
    } else {
        "x-webhook-secret"
    })
    .execute(pool)
    .await
    .expect("seed gateway brokered inbound rule");
    GatewayGoldenFixture {
        mailbox_id,
        grant_id,
    }
}
