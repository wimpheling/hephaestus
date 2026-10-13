use super::{Snapshot, consume, counts, exact, fields, indexed, records};
use serde_json::{Value, json};

pub(super) fn relations141(before: &Snapshot, after: &Snapshot) {
    let old = indexed(before, "relations");
    let current = indexed(after, "relations");
    assert!(old.keys().eq(current.keys()));
    let mut changed = 0;
    for (oid, original) in old {
        let following = current[oid];
        if original.value["identity"]["identity"] == "public.secret_runtime_mounts" {
            assert_secret_delta(original.value, following.value);
            let mut expected = fields(original);
            let actual = fields(following);
            assert!(
                expected
                    .insert("raw.relacl".to_owned(), actual["raw.relacl"])
                    .is_some()
            );
            assert_eq!(expected, actual, "only Source141 relacl changes");
            changed += 1;
        } else {
            exact(original, following);
        }
    }
    assert_eq!(changed, 1);
}

fn assert_secret_delta(original: &Value, following: &Value) {
    assert_eq!(original["owner"], "postgres");
    assert_eq!(original["namespace"], "public");
    assert_eq!(original["raw"]["relkind"], "r");
    assert!(original["raw"]["relacl"].is_null());
    let mut expected = original.clone();
    expected["raw"]["relacl"] = json!([
        "postgres=arwdDxtm/postgres",
        "hephaestus_worker=arw/postgres"
    ]);
    assert_eq!(
        &expected, following,
        "only worker SIU, no owner/OID/security drift"
    );
}

pub(super) fn shared141(before: &Snapshot, after: &Snapshot) {
    let current = records(after, "shared_dependencies");
    let mut remaining = counts(&current);
    for record in records(before, "shared_dependencies") {
        consume(&mut remaining, record.text);
    }
    let added: Vec<_> = current
        .iter()
        .filter(|record| remaining[record.text] > 0)
        .collect();
    assert_eq!(added.len(), 1, "exact single Source141 ACL edge");
    let secret: Vec<_> = records(before, "relations")
        .into_iter()
        .filter(|record| record.value["identity"]["identity"] == "public.secret_runtime_mounts")
        .collect();
    let worker: Vec<_> = before.domains()["roles"]
        .json()
        .as_array()
        .unwrap()
        .iter()
        .filter(|role| role["rolname"] == "hephaestus_worker")
        .collect();
    let database = before.domains()["database"].json().as_array().unwrap();
    assert_eq!((secret.len(), worker.len(), database.len()), (1, 1, 1));
    let edge = added[0].value;
    assert_eq!(
        edge["raw"],
        json!({"dbid":database[0]["raw"]["oid"],"classid":"1259",
        "objid":secret[0].value["raw"]["oid"],"objsubid":0,"refclassid":"1260",
        "refobjid":worker[0]["oid"],"deptype":"a"})
    );
    assert_eq!(edge["object"], secret[0].value["identity"]);
    assert_eq!(
        edge["reference"],
        json!({"type":"role","schema":null,"name":"hephaestus_worker","identity":"hephaestus_worker"})
    );
}

#[test]
fn forward_secret_acl_exception_rejects_delete_owner_and_identity_drift() {
    let original = json!({"namespace":"public","owner":"postgres","identity":{"identity":"public.secret_runtime_mounts"},
        "raw":{"oid":"17000","relkind":"r","relowner":"10","relacl":null,"relrowsecurity":false}});
    let mut expected = original.clone();
    expected["raw"]["relacl"] = json!([
        "postgres=arwdDxtm/postgres",
        "hephaestus_worker=arw/postgres"
    ]);
    assert_secret_delta(&original, &expected);
    for (field, value) in [
        (
            "relacl",
            json!([
                "postgres=arwdDxtm/postgres",
                "hephaestus_worker=arwd/postgres"
            ]),
        ),
        ("relowner", json!("20")),
        ("oid", json!("17001")),
        ("relrowsecurity", json!(true)),
    ] {
        let mut drift = expected.clone();
        drift["raw"][field] = value;
        assert!(
            std::panic::catch_unwind(|| assert_secret_delta(&original, &drift)).is_err(),
            "must reject {field}"
        );
    }
}
