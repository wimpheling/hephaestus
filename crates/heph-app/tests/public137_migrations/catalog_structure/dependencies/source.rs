//! Finite changes obtained from existing source-qualified metadata policies.
use super::{edges::Edge, identity, objects::Objects};
use serde_json::Value;
use std::collections::BTreeSet;

type Key = (String, String);
pub(super) struct Changes {
    retired_or_rebuilt: BTreeSet<Key>,
    replaced_routines: BTreeSet<Key>,
    rewritten: BTreeSet<Key>,
    worker: String,
    granted_routine: String,
    granted_relation: String,
}

impl Changes {
    pub fn new(
        old: &Objects<'_>,
        current: &Objects<'_>,
        fresh: &Objects<'_>,
        rewritten: &BTreeSet<String>,
    ) -> Self {
        let constraint_keys = super::super::super::catalog_constraints::dependency_changed_keys(
            old.structure.snapshot,
        );
        let mut retired_or_rebuilt: BTreeSet<_> = constraint_keys
            .iter()
            .map(|key| ("2606".to_owned(), key.clone()))
            .collect();
        for (key, record) in super::super::triggers::dependency_inventory(old.structure) {
            let id = identity::oid(&record.value["raw"]["tgconstraint"]);
            if old
                .bindings
                .get(&("2606".to_owned(), id.to_owned(), 0))
                .is_some_and(|binding| constraint_keys.contains(&binding.key))
            {
                assert!(key.starts_with("RI("));
                retired_or_rebuilt.insert(("2620".to_owned(), key));
            }
        }
        let removed = "public.one_active_instance_run_lease";
        assert!(old.structure.relations.contains_key(removed));
        assert!(
            !current.structure.relations.contains_key(removed)
                && !fresh.structure.relations.contains_key(removed)
        );
        retired_or_rebuilt.insert(("1259".to_owned(), removed.to_owned()));
        let policy: Value =
            serde_json::from_str(include_str!("../../catalog_routines/policy.json")).unwrap();
        let mut replaced_routines = BTreeSet::new();
        for declaration in policy["declared"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["replaced"] == true)
        {
            let matches: Vec<_> = identity::rows(old.structure.snapshot, "routines")
                .iter()
                .filter(|routine| {
                    routine["raw"]["proname"] == declaration["name"]
                        && routine["address"]["object_args"] == declaration["input_types"]
                })
                .collect();
            assert_eq!(matches.len(), 1, "qualified replaced routine signature");
            replaced_routines.insert((
                "1255".to_owned(),
                identity::text(&matches[0]["identity"]["identity"]).to_owned(),
            ));
        }
        assert_eq!(replaced_routines.len(), 7);
        let rewritten = old
            .bindings
            .iter()
            .filter(|((class, id, _), _)| {
                class == "1259"
                    && old
                        .structure
                        .classes
                        .get(id)
                        .is_some_and(|key| rewritten.contains(key))
            })
            .map(|((class, _, _), binding)| (class.clone(), binding.key.clone()))
            .collect();
        let workers: Vec<_> = identity::rows(old.structure.snapshot, "roles")
            .iter()
            .filter(|role| role["rolname"] == "hephaestus_worker")
            .collect();
        assert_eq!(workers.len(), 1);
        let routines: Vec<_> = identity::rows(old.structure.snapshot, "routines")
            .iter()
            .filter(|routine| {
                routine["raw"]["proname"] == "assert_instance_accepts_work"
                    && routine["address"]["object_args"] == serde_json::json!(["pg_catalog.uuid"])
            })
            .collect();
        assert_eq!(routines.len(), 1);
        Self {
            retired_or_rebuilt,
            replaced_routines,
            rewritten,
            worker: identity::oid(&workers[0]["oid"]).to_owned(),
            granted_routine: identity::text(&routines[0]["identity"]["identity"]).to_owned(),
            granted_relation: "public.secret_runtime_mounts".to_owned(),
        }
    }

    pub fn changed(&self, edge: &Edge<'_>) -> bool {
        self.retired_or_rebuilt.contains(&edge.object)
            || self.retired_or_rebuilt.contains(&edge.reference)
            || (edge.kind == "n" && self.replaced_routines.contains(&edge.object))
    }

    pub fn rewritten(&self, edge: &Edge<'_>) -> bool {
        self.rewritten.contains(&edge.object) || self.rewritten.contains(&edge.reference)
    }

    pub fn granted(&self, edge: &Edge<'_>) -> bool {
        edge.kind == "a"
            && (edge.object == ("1255".to_owned(), self.granted_routine.clone())
                || edge.object == ("1259".to_owned(), self.granted_relation.clone()))
            && edge.record.value["raw"]["refclassid"] == "1260"
            && edge.record.value["raw"]["refobjid"] == self.worker
    }
}
