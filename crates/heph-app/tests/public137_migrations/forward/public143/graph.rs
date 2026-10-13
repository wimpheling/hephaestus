use super::{Endpoint, Record, Snapshot, TABLE, additions, exact, indexed, records, relation_ids};
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn declared_relations(after: &Snapshot) -> Vec<Record<'_>> {
    let all = indexed(after, "relations");
    let parents: Vec<_> = all
        .values()
        .filter(|r| r.value["identity"]["identity"] == TABLE)
        .copied()
        .collect();
    assert_eq!(parents.len(), 1);
    let parent = parents[0];
    let raw = &parent.value["raw"];
    assert_eq!(raw["relnatts"], 32);
    assert_eq!(raw["relchecks"], 15);
    assert_eq!(raw["relrowsecurity"], true);
    assert_eq!(raw["relforcerowsecurity"], true);
    assert_eq!(parent.value["owner"], "postgres");
    let toast = all[raw["reltoastrelid"].as_str().unwrap()];
    assert_eq!(toast.value["raw"]["relkind"], "t");
    assert_eq!(
        toast.value["raw"]["relname"],
        format!("pg_toast_{}", raw["oid"].as_str().unwrap())
    );
    let mut result = vec![parent, toast];
    let indexes: Vec<_> = records(after, "indexes")
        .into_iter()
        .filter(|r| {
            r.value["raw"]["indrelid"] == raw["oid"]
                || r.value["raw"]["indrelid"] == toast.value["raw"]["oid"]
        })
        .collect();
    assert_eq!(indexes.len(), 9);
    let constraints = indexed(after, "constraints");
    let supported: BTreeSet<_> = constraints
        .values()
        .filter(|r| {
            r.value["raw"]["conrelid"] == raw["oid"]
                && matches!(r.value["raw"]["contype"].as_str().unwrap(), "p" | "u")
        })
        .map(|r| r.value["raw"]["conindid"].as_str().unwrap())
        .collect();
    assert_eq!(supported.len(), 8);
    for index in indexes {
        let id = index.value["raw"]["indexrelid"].as_str().unwrap();
        let relation = all[id];
        assert_eq!(relation.value["raw"]["relkind"], "i");
        if index.value["raw"]["indrelid"] == raw["oid"] {
            assert!(supported.contains(id));
        } else {
            assert_eq!(
                relation.value["raw"]["relname"],
                format!("pg_toast_{}_index", raw["oid"].as_str().unwrap())
            );
            assert_eq!(index.value["raw"]["indisunique"], true);
        }
        result.push(relation);
    }
    assert_eq!(result.len(), 11);
    result
}
pub(super) fn relations(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let new = additions(before, after, "relations");
    let declared = declared_relations(after);
    assert_eq!(new.len(), 11);
    let expected: BTreeSet<_> = declared
        .iter()
        .map(|r| r.value["raw"]["oid"].as_str().unwrap())
        .collect();
    assert_eq!(
        new.iter()
            .map(|r| r.value["raw"]["oid"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        expected
    );
    for record in new {
        assert!(added.insert((
            "1259".to_owned(),
            record.value["raw"]["oid"].as_str().unwrap().to_owned(),
            0
        )));
    }
}
pub(super) fn types(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let new = additions(before, after, "types");
    assert_eq!(new.len(), 2);
    let relations = declared_relations(after);
    let parent = relations[0];
    let row_id = parent.value["raw"]["reltype"].as_str().unwrap();
    let row = new
        .iter()
        .find(|r| r.value["raw"]["oid"] == row_id)
        .unwrap();
    assert_eq!(row.value["raw"]["typrelid"], parent.value["raw"]["oid"]);
    let array_id = row.value["raw"]["typarray"].as_str().unwrap();
    let array = new
        .iter()
        .find(|r| r.value["raw"]["oid"] == array_id)
        .unwrap();
    assert_eq!(array.value["raw"]["typelem"], row.value["raw"]["oid"]);
    for record in new {
        assert!(added.insert((
            "1247".to_owned(),
            record.value["raw"]["oid"].as_str().unwrap().to_owned(),
            0
        )));
    }
}
fn attributes(snapshot: &Snapshot) -> BTreeMap<(String, i64), Record<'_>> {
    let mut result = BTreeMap::new();
    for r in records(snapshot, "columns") {
        let raw = &r.value["raw"];
        assert!(
            result
                .insert(
                    (
                        raw["attrelid"].as_str().unwrap().to_owned(),
                        raw["attnum"].as_i64().unwrap()
                    ),
                    r
                )
                .is_none()
        );
    }
    result
}
pub(super) fn columns(before: &Snapshot, after: &Snapshot, added: &mut BTreeSet<Endpoint>) {
    let old = attributes(before);
    let current = attributes(after);
    for (key, original) in &old {
        exact(*original, current[key]);
    }
    let allowed = relation_ids(after);
    let mut count = 0;
    for (key, record) in current.iter().filter(|(key, _)| !old.contains_key(*key)) {
        assert!(allowed.contains(&key.0));
        assert_eq!(record.value["raw"]["attisdropped"], false);
        assert!(added.insert(("1259".to_owned(), key.0.clone(), key.1)));
        count += 1;
    }
    assert_eq!(count, 57);
}
pub(super) fn indexes(before: &Snapshot, after: &Snapshot) {
    let old: BTreeMap<_, _> = records(before, "indexes")
        .into_iter()
        .map(|r| (r.value["raw"]["indexrelid"].as_str().unwrap(), r))
        .collect();
    let current: BTreeMap<_, _> = records(after, "indexes")
        .into_iter()
        .map(|r| (r.value["raw"]["indexrelid"].as_str().unwrap(), r))
        .collect();
    assert_eq!(current.len(), old.len() + 9);
    for (id, original) in &old {
        exact(*original, current[id]);
    }
    let allowed = relation_ids(after);
    for (id, record) in current.iter().filter(|(id, _)| !old.contains_key(*id)) {
        assert!(allowed.contains(*id));
        assert!(allowed.contains(record.value["raw"]["indrelid"].as_str().unwrap()));
        for field in ["indisvalid", "indisready", "indislive", "indisunique"] {
            assert_eq!(record.value["raw"][field], true);
        }
        assert_eq!(record.value["raw"]["indcheckxmin"], false);
    }
}
