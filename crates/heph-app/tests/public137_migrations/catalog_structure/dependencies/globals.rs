use super::{Record, Structure, catalog, identity, records, signed};
use std::collections::BTreeMap;

pub(super) fn compare(old: &Structure<'_>, current: &Structure<'_>, fresh: &Structure<'_>) {
    let original = inventory(old);
    assert_eq!(
        original,
        inventory(current),
        "global shared graph raw/text changed"
    );
    assert_eq!(
        original,
        inventory(fresh),
        "same-cluster fresh global graph raw/text differs"
    );
}

fn inventory(structure: &Structure<'_>) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, "shared_dependencies") {
        if record.value["raw"]["dbid"] != "0" {
            continue;
        }
        super::edges::shape(record, true);
        catalog(&record.value["raw"]["classid"]);
        signed(&record.value["raw"]["objsubid"]);
        if record.value["raw"]["refclassid"] == "1260" {
            let role: Vec<_> = identity::rows(structure.snapshot, "roles")
                .iter()
                .filter(|entry| entry["oid"] == record.value["raw"]["refobjid"])
                .collect();
            assert!(role.len() <= 1);
            if let Some(role) = role.first() {
                super::bindings::role(&record.value["reference"], role);
            }
        }
        count(&mut result, record);
    }
    result
}

pub(super) fn count(result: &mut BTreeMap<String, usize>, record: Record<'_>) {
    // Includes every raw/helper byte and duplicate row; no peer object binding.
    *result.entry(record.text.to_owned()).or_default() += 1;
}
