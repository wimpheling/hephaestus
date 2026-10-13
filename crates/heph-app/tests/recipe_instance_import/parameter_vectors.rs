use std::collections::BTreeMap;

use recipe_application::InstallProgress;
use release_domain::{
    ParameterDeclaration, ParameterDocument, ParameterName, ParameterType, ParameterValue,
};
use sqlx::PgPool;

use crate::{fixtures, instance_fixture::Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn prepared107_import_persists_exact_framed_parameter_hash_for_sorted_defaults_and_all_value_types(
    pool: PgPool,
) {
    // Reverse declared order contrasts with canonical ParameterName order.
    let declarations = vec![
        declaration(
            "zzz",
            ParameterType::Integer {
                minimum: i64::MIN,
                maximum: i64::MAX,
            },
            ParameterValue::Integer(i64::MAX),
        ),
        declaration(
            "minimum",
            ParameterType::Integer {
                minimum: i64::MIN,
                maximum: i64::MAX,
            },
            ParameterValue::Integer(i64::MIN),
        ),
        declaration(
            "true_value",
            ParameterType::Boolean,
            ParameterValue::Boolean(true),
        ),
        declaration(
            "false_value",
            ParameterType::Boolean,
            ParameterValue::Boolean(false),
        ),
        declaration(
            "aa",
            ParameterType::String {
                minimum_length: 0,
                maximum_length: 4096,
            },
            ParameterValue::String("é中🦀\"\\/\n\r\t\u{8}\u{c}\u{1}\u{1f}".to_owned()),
        ),
    ];
    let h = Harness::with_parameters(&pool, false, &declarations).await;
    let prepared = h.prepared().await;
    let document = ParameterDocument::resolve(&declarations, &BTreeMap::new()).unwrap();
    assert_eq!(prepared.parameters(), document.values());
    let result = h
        .importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    let (parameters, actual): (serde_json::Value, Vec<u8>) = sqlx::query_as(
        "SELECT parameters,parameter_hash FROM agent_instance_revisions WHERE id=$1",
    )
    .bind(prepared.revision_id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(parameters, serde_json::to_value(document.values()).unwrap());
    assert_eq!(
        actual,
        document.hash().as_bytes().as_slice(),
        "real107 revision hash must match its original typed-framing implementation"
    );
    let sql: Vec<u8> = sqlx::query_scalar("SELECT recipe_import107_parameter_hash($1)")
        .bind(parameters)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        sql, actual,
        "SQL comparison uses the actually committed107 revision, not a SQL-only expected value"
    );
}

fn declaration(
    name: &str,
    value_type: ParameterType,
    value: ParameterValue,
) -> ParameterDeclaration {
    ParameterDeclaration {
        name: ParameterName::parse(name).unwrap(),
        value_type,
        required: true,
        default: Some(value),
        sensitive: false,
    }
}
