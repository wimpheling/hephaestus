//! Secret lease fixture rows for execution-target tests.

use super::support_types::{SecretContext, TimingWindow};
use sqlx::PgPool;

// This fixture phase mirrors the full secret lease graph in one transaction setup.
#[allow(clippy::too_many_lines)]
pub async fn seed_secret(pool: &PgPool, context: SecretContext, secret_lease_timing: TimingWindow) {
    let SecretContext {
        organization,
        project,
        gateway,
        revision,
        route,
        owner_id,
        secret,
        version,
        grant,
        import,
        binding,
        rule,
        secret_lease,
        session,
        invocation,
    } = context;
    sqlx::query(
        "INSERT INTO secrets
                (id, owner_organization_id, project_id, name, status,
                 allowed_delivery_modes, policy, created_by)
             VALUES ($1, $2, $3, $4, 'active', ARRAY['brokered'], '{}', $5)",
    )
    .bind(secret)
    .bind(organization)
    .bind(project)
    .bind(format!("execution-secret-{secret}"))
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("secret");
    sqlx::query(
        "INSERT INTO secret_versions
                (id, secret_id, sequence, status, algorithm, key_reference,
                 data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
                 associated_data_hash, content_length, created_by)
             VALUES ($1, $2, 1, 'active', 'AES-256-GCM+AES-256-GCM-KW/v1',
                     'execution-key', $3, $4, $5, $6, $7, 1, $8)",
    )
    .bind(version)
    .bind(secret)
    .bind([1_u8; 12].as_slice())
    .bind([2_u8; 1].as_slice())
    .bind([3_u8; 12].as_slice())
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("secret version");
    sqlx::query("UPDATE secrets SET active_version_id = $2 WHERE id = $1")
        .bind(secret)
        .bind(version)
        .execute(pool)
        .await
        .expect("active version");
    sqlx::query(
        "INSERT INTO secret_grants
                (id, secret_id, owner_organization_id, target_kind, target_id,
                 target_project_id, delivery_modes, phases, status, created_by)
             VALUES ($1, $2, $3, 'project', $4, $4, ARRAY['brokered'],
                     ARRAY['normal'], 'active', $5)",
    )
    .bind(grant)
    .bind(secret)
    .bind(organization)
    .bind(project)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("secret grant");
    sqlx::query(
        "INSERT INTO secret_imports
                (id, grant_id, secret_id, target_kind, target_id, alias,
                 status, accepted_by)
             VALUES ($1, $2, $3, 'project', $4, $5, 'active', $6)",
    )
    .bind(import)
    .bind(grant)
    .bind(secret)
    .bind(project)
    .bind(format!("execution-import-{import}"))
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("secret import");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
                (id, gateway_id, gateway_revision_id, import_id, slot_key,
                 secret_version_id, status, normalized_hash)
             VALUES ($1, $2, $3, $4, 'hook', $5, 'active', $6)",
    )
    .bind(binding)
    .bind(gateway)
    .bind(revision)
    .bind(import)
    .bind(version)
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("gateway secret binding");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
                (id, binding_id, gateway_revision_id, gateway_route_id,
                 header_name, normalized_hash)
             VALUES ($1, $2, $3, $4, 'x-hook', $5)",
    )
    .bind(rule)
    .bind(binding)
    .bind(revision)
    .bind(route)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("gateway secret rule");
    let lease_query = match secret_lease_timing {
        TimingWindow::Live => sqlx::query(
            "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active', now(),
                         now() + interval '5 minutes')",
        ),
        TimingWindow::Near => sqlx::query(
            "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active', now(),
                         now() + interval '1 millisecond')",
        ),
        TimingWindow::Expired => sqlx::query(
            "INSERT INTO gateway_secret_leases
                    (id, runtime_session_id, invocation_id, binding_id,
                     secret_version_id, rule_id, status, issued_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, 'active',
                         now() - interval '2 minutes',
                         now() - interval '1 minute')",
        ),
    };
    lease_query
        .bind(secret_lease)
        .bind(session)
        .bind(invocation)
        .bind(binding)
        .bind(version)
        .bind(rule)
        .execute(pool)
        .await
        .expect("gateway secret lease");
}
