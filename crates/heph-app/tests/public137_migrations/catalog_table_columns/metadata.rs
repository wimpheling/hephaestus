use super::super::catalog::Snapshot;
use super::json_text;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const RAW_FIELDS: [&str; 26] = [
    "attrelid",
    "attname",
    "atttypid",
    "attlen",
    "attnum",
    "attcacheoff",
    "atttypmod",
    "attndims",
    "attbyval",
    "attalign",
    "attstorage",
    "attcompression",
    "attnotnull",
    "atthasdef",
    "atthasmissing",
    "attidentity",
    "attgenerated",
    "attisdropped",
    "attislocal",
    "attinhcount",
    "attcollation",
    "attstattarget",
    "attacl",
    "attoptions",
    "attfdwoptions",
    "attmissingval",
];

pub struct Column<'a> {
    value: &'a Value,
    text: &'a str,
}

impl Column<'_> {
    pub const fn value(&self) -> &Value {
        self.value
    }
    pub const fn text(&self) -> &str {
        self.text
    }
}

pub struct Table<'a> {
    pub oid: &'a str,
    pub columns: BTreeMap<i64, Column<'a>>,
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

pub fn text(value: &Value) -> &str {
    value.as_str().expect("expected captured text")
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

fn oid(value: &Value) -> &str {
    let value = text(value);
    assert!(!value.is_empty() && !value.starts_with('0'));
    assert!(value.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(value.parse::<u32>().expect("OID fits u32") > 0);
    value
}

pub fn tables(snapshot: &Snapshot) -> BTreeMap<String, Table<'_>> {
    let mut tables = BTreeMap::new();
    let mut by_oid = BTreeMap::new();
    let mut expected_counts = BTreeMap::new();
    for relation in rows(snapshot, "relations") {
        let raw = &relation["raw"];
        if relation["namespace"] != "public" || raw["relkind"] != "r" {
            continue;
        }
        let name = text(&raw["relname"]);
        assert_eq!(relation["identity"]["schema"], "public");
        assert_eq!(relation["identity"]["name"], name);
        let id = oid(&raw["oid"]);
        assert!(by_oid.insert(id, relation).is_none());
        assert!(
            expected_counts
                .insert(name, raw["relnatts"].as_i64().unwrap())
                .is_none()
        );
        assert!(
            tables
                .insert(
                    name.to_owned(),
                    Table {
                        oid: id,
                        columns: BTreeMap::new()
                    }
                )
                .is_none()
        );
    }
    let domain = snapshot.domains().get("columns").expect("columns capture");
    let parsed = rows(snapshot, "columns");
    let original = json_text::array_items(domain.text());
    assert_eq!(original.len(), parsed.len());
    for (value, original) in parsed.iter().zip(original) {
        assert_eq!(&serde_json::from_str::<Value>(original).unwrap(), value);
        let raw = &value["raw"];
        let Some(relation) = by_oid.get(text(&raw["attrelid"])) else {
            continue;
        };
        let number = raw["attnum"].as_i64().expect("attribute number");
        if number <= 0 {
            continue;
        } // System attributes have a separate pending oracle.
        validate(value, original);
        assert_eq!(value["relation"], relation["identity"]);
        let table = tables.get_mut(text(&relation["raw"]["relname"])).unwrap();
        assert!(
            table
                .columns
                .insert(
                    number,
                    Column {
                        value,
                        text: original
                    }
                )
                .is_none()
        );
    }
    for (name, table) in &tables {
        let expected = expected_counts[name.as_str()];
        assert_eq!(
            table.columns.keys().copied().collect::<Vec<_>>(),
            (1..=expected).collect::<Vec<_>>()
        );
    }
    tables
}

fn validate(value: &Value, original: &str) {
    keys(
        value,
        &[
            "raw",
            "relation",
            "column_identity",
            "formatted_type",
            "type",
            "collation",
        ],
    );
    keys(&value["raw"], &RAW_FIELDS);
    for field in ["relation", "column_identity", "type", "collation"] {
        if !value[field].is_null() {
            keys(&value[field], &["type", "schema", "name", "identity"]);
        }
    }
    let raw = &value["raw"];
    oid(&raw["attrelid"]);
    assert!(raw["attnum"].as_i64().unwrap() > 0);
    assert!(matches!(text(&raw["attidentity"]), "" | "a" | "d"));
    assert!(matches!(text(&raw["attgenerated"]), "" | "s"));
    assert!(matches!(text(&raw["attcompression"]), "" | "p" | "l"));
    if raw["attisdropped"] == true {
        assert_eq!(raw["atttypid"], "0");
        for field in ["column_identity", "formatted_type", "type"] {
            assert!(value[field].is_null());
        }
    } else {
        assert_eq!(raw["attisdropped"], false);
        oid(&raw["atttypid"]);
        assert_eq!(value["column_identity"]["type"], "table column");
        assert_eq!(value["type"]["type"], "type");
        assert!(value["formatted_type"].is_string());
    }
    if raw["attcollation"] == "0" {
        assert!(value["collation"].is_null());
    } else {
        oid(&raw["attcollation"]);
        assert_eq!(value["collation"]["type"], "collation");
    }
    let original_raw = json_text::field(original, "raw");
    let names: BTreeSet<_> = RAW_FIELDS.into_iter().collect();
    assert_eq!(names.len(), RAW_FIELDS.len());
    // Traverse every expected field, rejecting duplicates/missing keys in raw text.
    for name in RAW_FIELDS {
        json_text::field(original_raw, name);
    }
}

pub fn normalized(column: &Column<'_>) -> Value {
    let mut value = column.value().clone();
    let original_raw = json_text::field(column.text(), "raw");
    value["raw"]["attmissingval"] =
        json!({"exact_PG_JSONB_token": json_text::field(original_raw, "attmissingval")});
    // Only these three typed OID fields change representation across databases.
    value["raw"]["attrelid"] = value["relation"].clone();
    if value["raw"]["atttypid"] != "0" {
        value["raw"]["atttypid"] = value["type"].clone();
    }
    if value["raw"]["attcollation"] != "0" {
        value["raw"]["attcollation"] = value["collation"].clone();
    }
    value
}
