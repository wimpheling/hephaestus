//! Finite normal pgcrypto1.3 metadata profile. The caller qualifies actual
//! native assets and all three journals; metadata does not certify native files.
use super::{
    Inventory, Record, Semantic, Structure, exact, identity, json_text, records, replace, semantic,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[path = "extension_metadata/comments.rs"]
mod comments;
#[path = "extension_metadata/extensions.rs"]
mod extensions;
#[path = "extension_metadata/members.rs"]
mod members;
#[path = "extension_metadata/profile.rs"]
mod profile;
#[cfg(test)]
#[path = "extension_metadata/tests.rs"]
mod tests;

pub(super) fn compare(old: &Structure<'_>, upgraded: &Structure<'_>, fresh: &Structure<'_>) {
    profile::qualify();
    // This complete existing oracle independently admits the native roster,
    // symbols, languages, owners, ACLs and routine identities in all captures.
    super::super::catalog_routines::assert_routines(
        old.snapshot,
        upgraded.snapshot,
        fresh.snapshot,
    );
    let original = extensions::inventory(old);
    let current = extensions::inventory(upgraded);
    let reference = extensions::inventory(fresh);
    let expected = original
        .keys()
        .cloned()
        .chain(["pgcrypto".to_owned()])
        .collect();
    compare_domain(
        &original,
        &current,
        &reference,
        &expected,
        extensions::normalized,
        extensions::normalized,
    );
    members::compare(old, upgraded, fresh, &original, &current, &reference);
    comments::compare(old, upgraded, fresh, &original, &current, &reference);
}

fn compare_domain(
    old: &Inventory<'_>,
    upgraded: &Inventory<'_>,
    fresh: &Inventory<'_>,
    expected: &BTreeSet<String>,
    current_normalize: impl Fn(Record<'_>) -> Semantic,
    fresh_normalize: impl Fn(Record<'_>) -> Semantic,
) {
    assert_eq!(upgraded.keys().cloned().collect::<BTreeSet<_>>(), *expected);
    assert_eq!(fresh.keys().cloned().collect::<BTreeSet<_>>(), *expected);
    for (key, record) in old {
        exact(
            *record,
            upgraded[key],
            "complete inherited extension metadata",
        );
    }
    for key in expected {
        assert_eq!(
            current_normalize(upgraded[key]),
            fresh_normalize(fresh[key]),
            "canonical metadata {key}"
        );
    }
}

fn signed(value: &Value) -> i32 {
    i32::try_from(value.as_i64().expect("signed subobject identifier")).unwrap()
}

fn builtin(raw: &Value) {
    let id = identity::oid(raw).parse::<u32>().unwrap();
    assert!(id > 0 && id < 16384, "unresolved non-built-in object");
}

fn routine<'a>(structure: &'a Structure<'_>, raw: &Value, helper: &Value) -> &'a Value {
    let id = identity::oid(raw);
    let found: Vec<_> = identity::rows(structure.snapshot, "routines")
        .iter()
        .filter(|entry| entry["raw"]["oid"] == id)
        .collect();
    assert_eq!(found.len(), 1, "qualified captured routine link");
    assert_eq!(found[0]["identity"], *helper);
    found[0]
}

fn extension<'a>(inventory: &'a Inventory<'_>, raw: &Value) -> &'a str {
    let id = identity::oid(raw);
    let found: Vec<_> = inventory
        .iter()
        .filter(|(_, entry)| entry.value["raw"]["oid"] == id)
        .collect();
    assert_eq!(found.len(), 1, "unique extension OID link");
    found[0].0
}

// Only a checked extension OID is translated; all other embedded fields retain
// their original JSONB spelling, including arrays and NULLs.
fn embedded_extension(result: &mut Semantic, record: Record<'_>) {
    let value = &record.value["extension"];
    identity::keys(value, extensions::FIELDS);
    let text = json_text::field(record.text, "extension");
    assert!(result.remove("extension").is_some());
    for field in extensions::FIELDS {
        result.insert(
            format!("extension.{field}"),
            json_text::field(text, field).to_owned(),
        );
    }
    if identity::oid(&value["oid"]).parse::<u32>().unwrap() >= 16384 {
        replace(result, "extension.oid", &value["extname"]);
    }
}
