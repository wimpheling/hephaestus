use super::super::catalog::Snapshot;
use super::json_text;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct Default<'a> {
    pub value: &'a Value,
    pub text: &'a str,
    pub generated: bool,
}

pub fn rows<'a>(snapshot: &'a Snapshot, name: &str) -> &'a [Value] {
    snapshot
        .domains()
        .get(name)
        .expect("missing catalog domain")
        .json()
        .as_array()
        .expect("complete catalog array")
}

fn text(value: &Value) -> &str {
    value.as_str().expect("captured text")
}

fn oid(value: &Value) -> &str {
    let value = text(value);
    assert!(!value.is_empty() && !value.starts_with('0'));
    assert!(value.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(value.parse::<u32>().expect("OID fits u32") > 0);
    value
}

fn keys(value: &Value, expected: &[&str]) {
    let actual: BTreeSet<_> = value
        .as_object()
        .expect("captured object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(actual, expected.iter().copied().collect());
}

fn identity(value: &Value) {
    keys(value, &["type", "schema", "name", "identity"]);
    assert!(value["type"].is_string());
    assert!(value["identity"].is_string());
}

pub fn defaults(snapshot: &Snapshot) -> BTreeMap<String, Default<'_>> {
    let mut relations = BTreeMap::new();
    for relation in rows(snapshot, "relations") {
        assert!(
            relations
                .insert(oid(&relation["raw"]["oid"]), relation)
                .is_none()
        );
    }
    let mut columns = BTreeMap::new();
    for column in rows(snapshot, "columns") {
        let raw = &column["raw"];
        assert!(
            columns
                .insert(
                    (
                        oid(&raw["attrelid"]),
                        raw["attnum"].as_i64().expect("attribute number")
                    ),
                    column
                )
                .is_none()
        );
    }
    let capture = snapshot
        .domains()
        .get("defaults")
        .expect("defaults capture");
    let parsed = rows(snapshot, "defaults");
    let original = json_text::array_items(capture.text());
    assert_eq!(parsed.len(), original.len());
    let mut result = BTreeMap::new();
    let mut own_oids = BTreeSet::new();
    for (value, original) in parsed.iter().zip(original) {
        assert_eq!(&serde_json::from_str::<Value>(original).unwrap(), value);
        keys(value, &["raw", "relation", "column", "expression"]);
        let raw = &value["raw"];
        keys(raw, &["oid", "adrelid", "adnum", "adbin"]);
        let raw_text = json_text::field(original, "raw");
        for field in ["oid", "adrelid", "adnum", "adbin"] {
            json_text::field(raw_text, field);
        }
        assert!(own_oids.insert(oid(&raw["oid"])));
        let parent_oid = oid(&raw["adrelid"]);
        let number = raw["adnum"].as_i64().expect("default attribute number");
        assert!(number > 0 && i16::try_from(number).is_ok());
        assert!(raw["adbin"].is_string());
        assert!(value["expression"].is_string());
        identity(&value["relation"]);
        identity(&value["column"]);
        let relation = relations.get(parent_oid).expect("default parent captured");
        let column = columns
            .get(&(parent_oid, number))
            .expect("default column captured");
        assert_eq!(value["relation"], relation["identity"]);
        assert_eq!(value["column"], column["column_identity"]);
        assert_eq!(column["raw"]["attisdropped"], false);
        assert_eq!(column["raw"]["atthasdef"], true);
        let generated = match text(&column["raw"]["attgenerated"]) {
            "" => false,
            "s" => true,
            _ => panic!("unsupported generated default kind"),
        };
        let name = format!(
            "{}.{}.{}",
            text(&relation["namespace"]),
            text(&relation["raw"]["relname"]),
            text(&column["raw"]["attname"])
        );
        assert!(
            result
                .insert(
                    name,
                    Default {
                        value,
                        text: original,
                        generated
                    }
                )
                .is_none()
        );
    }
    result
}

pub fn normalized(default: &Default<'_>) -> Value {
    let mut value = default.value.clone();
    // These are typed pg_attrdef row and parent-relation identity fields only.
    // adnum, the entire adbin node tree and expression deparser remain exact.
    value["raw"]["oid"] = json!({"catalog": "pg_attrdef", "column": value["column"].clone()});
    value["raw"]["adrelid"] = value["relation"].clone();
    value
}
