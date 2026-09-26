use serde_json::json;
use uuid::Uuid;

use super::Context;
use crate::support::{
    assert_sqlstate, commit, insert_invalid_non_blob, insert_invalid_oversized,
    insert_valid_revision,
};

pub async fn run(context: &Context) {
    let fixture = context.fixture;
    insert_valid_revision(
        &context.worker,
        fixture.public_repository,
        fixture.public_receive,
        fixture.public_revision,
    )
    .await;
    insert_valid_revision(
        &context.worker,
        fixture.private_repository,
        fixture.private_receive,
        fixture.private_revision,
    )
    .await;
    insert_invalid_oversized(&context.worker, &fixture).await;
    insert_invalid_non_blob(&context.worker, &fixture).await;

    let invalid_observation: (i64, bool) = sqlx::query_as(
        "SELECT actual_size_bytes, normalized_ui_config IS NULL
         FROM ui_source_manifest_revisions
         WHERE id = $1",
    )
    .bind(fixture.oversized_revision)
    .fetch_one(&context.worker)
    .await
    .expect("invalid observation");
    assert_eq!(invalid_observation.0, 262_145);
    assert!(
        invalid_observation.1,
        "invalid rows have no authoritative UI JSON"
    );

    assert_gateway_constraints(context, fixture).await;
    let visible_valid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND status = 'valid'",
    )
    .bind(fixture.public_repository)
    .fetch_one(&context.admin)
    .await
    .expect("valid source revision");
    assert_eq!(visible_valid, 1);
}

async fn assert_gateway_constraints(context: &Context, fixture: crate::support::Fixture) {
    let incomplete_gateway = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', true, $7, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("e"))
    .bind(commit("f"))
    .bind([5_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([6_u8; 32].as_slice())
    .execute(&context.worker)
    .await;
    assert!(
        incomplete_gateway.is_err(),
        "gateway-referencing valid rows require a complete gateway snapshot"
    );
    assert_sqlstate(incomplete_gateway, "23514");

    let oversized_gateway = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash, gateway_manifest_oid,
          gateway_actual_size_bytes, gateway_source_sha256,
          normalized_gateway_config, normalized_gateway_hash)
         VALUES ($1, $2, $3, $4, 'blob', $5, 32, $6, 'valid', true,
                 $7, $8, $9, 1048577, $10, $11, $12)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("g"))
    .bind(commit("h"))
    .bind([7_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([8_u8; 32].as_slice())
    .bind(commit("i"))
    .bind([9_u8; 32].as_slice())
    .bind(json!({"version": 1, "routes": []}))
    .bind([10_u8; 32].as_slice())
    .execute(&context.worker)
    .await;
    assert!(
        oversized_gateway.is_err(),
        "valid gateway snapshots have a 1 MiB source cap"
    );
    assert_sqlstate(oversized_gateway, "23514");

    let missing_oid = sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind,
          status, diagnostics)
         VALUES ($1, $2, $3, $4, 'tree', 'invalid', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.public_repository)
    .bind(fixture.public_receive)
    .bind(commit("j"))
    .bind(json!([{"code": "ui_manifest_missing_oid"}]))
    .execute(&context.worker)
    .await;
    assert!(
        missing_oid.is_err(),
        "every present entry must retain its OID"
    );
    assert_sqlstate(missing_oid, "23502");
}
