use super::fixtures::Fixture;
use uuid::Uuid;

// This helper intentionally creates the full secret/binding/rule/lease chain
// needed to exercise the database triggers rather than bypassing them.
#[allow(clippy::too_many_lines)]
pub async fn seed_gateway_lease(pool: &sqlx::PgPool, fixture: &Fixture) -> Uuid {
    let secret = Uuid::new_v4();
    let version = Uuid::new_v4();
    let grant = Uuid::new_v4();
    let import = Uuid::new_v4();
    let binding = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let lease = Uuid::new_v4();
    let name = format!("runtime_{:032x}", secret.as_u128());
    sqlx::query(
        "INSERT INTO secrets
            (id, owner_organization_id, project_id, name, status,
             allowed_delivery_modes, created_by)
         VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], $5)",
    )
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(name)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret");
    sqlx::query(
        "INSERT INTO secret_versions
            (id, secret_id, sequence, status, algorithm, key_reference,
             data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
             associated_data_hash, content_length, created_by)
         VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                 'test/key', decode(repeat('01', 12), 'hex'),
                 decode('01', 'hex'), decode(repeat('02', 12), 'hex'),
                 decode('03', 'hex'), decode(repeat('04', 32), 'hex'), 1, $3)",
    )
    .bind(version)
    .bind(secret)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret version");
    sqlx::query("UPDATE secrets SET active_version_id = $2 WHERE id = $1")
        .bind(secret)
        .bind(version)
        .execute(pool)
        .await
        .expect("active gateway secret version");
    sqlx::query(
        "INSERT INTO secret_grants
            (id, secret_id, owner_organization_id, target_kind, target_id,
             target_project_id, delivery_modes, phases, status, created_by)
         VALUES ($1, $2, $3, 'project', $4, $4, ARRAY['brokered'],
                 ARRAY['normal'], 'active', $5)",
    )
    .bind(grant)
    .bind(secret)
    .bind(fixture.organization)
    .bind(fixture.project)
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret grant");
    sqlx::query(
        "INSERT INTO secret_imports
            (id, grant_id, secret_id, target_kind, target_id, alias, status,
             accepted_by)
         VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6)",
    )
    .bind(import)
    .bind(grant)
    .bind(secret)
    .bind(fixture.project)
    .bind(format!("runtime_{:032x}", import.as_u128()))
    .bind(fixture.owner)
    .execute(pool)
    .await
    .expect("gateway secret import");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
            (id, gateway_id, gateway_revision_id, import_id, slot_key,
             secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'hook', $5, 'active', $6)",
    )
    .bind(binding)
    .bind(fixture.gateway)
    .bind(fixture.revision)
    .bind(import)
    .bind(version)
    .bind([5_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("gateway secret binding");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
            (id, binding_id, gateway_revision_id, gateway_route_id,
             header_name, normalized_hash)
         SELECT $1, $2, $3, route.id, 'x-hook-secret', $4
           FROM gateway_routes AS route
          WHERE route.gateway_revision_id = $3",
    )
    .bind(rule)
    .bind(binding)
    .bind(fixture.revision)
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("gateway secret rule");
    sqlx::query(
        "INSERT INTO gateway_secret_leases
            (id, runtime_session_id, invocation_id, binding_id,
             secret_version_id, rule_id, status, expires_at)
         SELECT $1, session.id, session.invocation_id, $2, $3, $4,
                'active', session.expires_at
           FROM gateway_runtime_authority_sessions AS session
          WHERE session.id = $5",
    )
    .bind(lease)
    .bind(binding)
    .bind(version)
    .bind(rule)
    .bind(fixture.invocation)
    .execute(pool)
    .await
    .expect("gateway secret lease");
    lease
}
