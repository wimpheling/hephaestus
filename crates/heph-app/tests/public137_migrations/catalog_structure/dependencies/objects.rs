//! Captures are used only after their complete source-bound oracles passed.
use super::{Structure, bindings::Binding, identity, json, records, signed};
use serde_json::Value;
use std::collections::BTreeMap;

type Endpoint = (String, String, i32);
pub(super) struct Objects<'a> {
    pub structure: &'a Structure<'a>,
    pub bindings: BTreeMap<Endpoint, Binding>,
    pub database: String,
    native_routines: BTreeMap<String, Value>,
}

impl<'a> Objects<'a> {
    pub fn new(structure: &'a Structure<'a>) -> Self {
        let database = identity::rows(structure.snapshot, "database");
        assert_eq!(database.len(), 1);
        assert_eq!(
            database[0]["raw"]["datname"],
            structure.snapshot.context().json()["database"]
        );
        let mut result = Self {
            structure,
            bindings: BTreeMap::new(),
            native_routines: BTreeMap::new(),
            database: identity::oid(&database[0]["raw"]["oid"]).to_owned(),
        };
        assert_ne!(result.database, "0");
        result.classes();
        result.columns();
        result.local();
        result.named();
        result.triggers();
        result.defaults();
        result.extensions();
        result.native();
        result
    }

    fn insert(&mut self, class: &str, id: &Value, sub: i32, binding: Binding) {
        let oid = identity::oid(id);
        assert_ne!(oid, "0");
        assert!(
            self.bindings
                .insert((class.to_owned(), oid.to_owned(), sub), binding)
                .is_none(),
            "ambiguous endpoint"
        );
    }

    fn classes(&mut self) {
        let structure = self.structure;
        for (key, record) in &structure.relations {
            let raw = &record.value["raw"];
            let name = identity::text(&raw["relname"]);
            let helper = record.value["identity"].clone();
            let schema = identity::text(&helper["schema"]);
            let mut binding = Binding::new(
                key.clone(),
                helper.clone(),
                &[schema.to_owned(), name.to_owned()],
            );
            if schema == "pg_toast" {
                binding.canonical = identity::toast_identity(&helper, name, key);
                binding.canonical_address.as_mut().unwrap()["object_names"][1] = json!(key);
            }
            self.insert("1259", &raw["oid"], 0, binding);
        }
    }

    fn columns(&mut self) {
        let structure = self.structure;
        for record in records(structure.snapshot, "columns") {
            let raw = &record.value["raw"];
            if raw["attisdropped"] == true {
                continue;
            }
            let relation = &structure.classes[identity::oid(&raw["attrelid"])];
            let table = &structure.relations[relation].value;
            assert_eq!(record.value["relation"], table["identity"]);
            let sub = signed(&raw["attnum"]);
            assert_ne!(sub, 0);
            let name = identity::text(&raw["attname"]);
            let table_name = identity::text(&table["raw"]["relname"]);
            let schema = identity::text(&table["identity"]["schema"]);
            let helper = record.value["column_identity"].clone();
            let mut binding = Binding::new(
                format!("{relation}#{sub}"),
                helper.clone(),
                &[schema.to_owned(), table_name.to_owned(), name.to_owned()],
            );
            if schema == "pg_toast" {
                binding.canonical =
                    identity::toast_column_identity(&helper, table_name, name, relation);
                binding.canonical_address.as_mut().unwrap()["object_names"][1] = json!(relation);
            }
            self.insert("1259", &raw["attrelid"], sub, binding);
        }
    }

    fn local(&mut self) {
        let structure = self.structure;
        for (key, record) in &structure.types {
            let helper = record.value["identity"].clone();
            let names = vec![identity::text(&helper["identity"]).to_owned()];
            self.insert(
                "1247",
                &record.value["raw"]["oid"],
                0,
                Binding::new(key.clone(), helper, &names),
            );
        }
        for record in records(structure.snapshot, "routines") {
            let helper = record.value["identity"].clone();
            let key = identity::text(&helper["identity"]).to_owned();
            let mut binding = Binding::new(key, helper, &[]);
            binding.address = Some(record.value["address"].clone());
            binding.canonical_address.clone_from(&binding.address);
            self.insert("1255", &record.value["raw"]["oid"], 0, binding);
        }
    }

    fn named(&mut self) {
        let structure = self.structure;
        for (domain, class, table_field, name_field) in [
            ("constraints", "2606", "conrelid", "conname"),
            ("policies", "3256", "polrelid", "polname"),
            ("rules", "2618", "ev_class", "rulename"),
        ] {
            for record in records(structure.snapshot, domain) {
                let raw = &record.value["raw"];
                let relation = &structure.classes[identity::oid(&raw[table_field])];
                let name = identity::text(&raw[name_field]);
                let names = vec![
                    "public".to_owned(),
                    identity::text(&structure.relations[relation].value["raw"]["relname"])
                        .to_owned(),
                    name.to_owned(),
                ];
                self.insert(
                    class,
                    &raw["oid"],
                    0,
                    Binding::new(
                        format!("{relation}.{name}"),
                        record.value["identity"].clone(),
                        &names,
                    ),
                );
            }
        }
    }

    fn triggers(&mut self) {
        let structure = self.structure;
        for (key, record) in super::super::triggers::dependency_inventory(structure) {
            let raw = &record.value["raw"];
            let relation = &structure.classes[identity::oid(&raw["tgrelid"])];
            let names = vec![
                "public".to_owned(),
                identity::text(&structure.relations[relation].value["raw"]["relname"]).to_owned(),
                identity::text(&raw["tgname"]).to_owned(),
            ];
            let mut binding = Binding::new(key.clone(), record.value["identity"].clone(), &names);
            if key.starts_with("RI(") {
                binding.canonical["identity"] = json!(format!(
                    "\"{key}\" on {}",
                    identity::text(&record.value["relation"]["identity"])
                ));
                binding.canonical_address.as_mut().unwrap()["object_names"][2] = json!(key);
            }
            self.insert("2620", &raw["oid"], 0, binding);
        }
    }

    fn defaults(&mut self) {
        let structure = self.structure;
        for record in records(structure.snapshot, "defaults") {
            let raw = &record.value["raw"];
            let column = &self.bindings[&(
                "1259".to_owned(),
                identity::oid(&raw["adrelid"]).to_owned(),
                signed(&raw["adnum"]),
            )];
            assert_eq!(column.helper, record.value["column"]);
            let helper = json!({"type":"default value","schema":null,"name":null,"identity":format!("for {}",identity::text(&column.helper["identity"]))});
            let names: Vec<String> = column.address.as_ref().unwrap()["object_names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| identity::text(value).to_owned())
                .collect();
            self.insert(
                "2604",
                &raw["oid"],
                0,
                Binding::new(format!("default({})", column.key), helper, &names),
            );
        }
    }

    fn extensions(&mut self) {
        let structure = self.structure;
        for record in records(structure.snapshot, "extensions") {
            let raw = &record.value["raw"];
            let name = identity::text(&raw["extname"]);
            let helper = json!({"type":"extension","schema":null,"name":name,"identity":name});
            self.insert(
                "3079",
                &raw["oid"],
                0,
                Binding::new(name.to_owned(), helper, &[name.to_owned()]),
            );
        }
    }

    fn native(&mut self) {
        let structure = self.structure;
        for language in identity::rows(structure.snapshot, "languages") {
            for (field, helper_field) in [
                ("lanplcallfoid", "call_handler"),
                ("laninline", "inline_handler"),
                ("lanvalidator", "validator"),
            ] {
                let id = &language["raw"][field];
                if identity::oid(id) == "0" {
                    continue;
                }
                let helper = &language[helper_field];
                // PG17 native handler IDs stay literal, including plpgsql's qualified four-member roster.
                assert!(
                    identity::oid(id).parse::<u32>().unwrap() < 16384,
                    "native handler OID"
                );
                identity::object(helper);
                assert_eq!(helper["schema"], "pg_catalog");
                if let Some(previous) = self
                    .native_routines
                    .insert(identity::oid(id).to_owned(), helper.clone())
                {
                    assert_eq!(previous, *helper, "unique native routine witness");
                }
            }
        }
    }

    pub fn endpoint(
        &self,
        class: &str,
        id: &str,
        sub: i32,
        helper: &Value,
        address: Option<&Value>,
    ) -> (String, Value, Option<Value>) {
        if let Some(binding) = self.bindings.get(&(class.to_owned(), id.to_owned(), sub)) {
            binding.check(helper, address);
            return (
                binding.key.clone(),
                binding.canonical.clone(),
                binding.canonical_address.clone(),
            );
        }
        if class == "1255"
            && let Some(native) = self.native_routines.get(id)
        {
            assert_eq!(*native, *helper, "native language handler witness");
        }
        if class == "2612" {
            let languages: Vec<_> = identity::rows(self.structure.snapshot, "languages")
                .iter()
                .filter(|entry| entry["raw"]["oid"] == id)
                .collect();
            assert_eq!(languages.len(), 1, "native language witness");
            assert_eq!(helper["type"], "language");
            assert!(helper["schema"].is_null());
            assert_eq!(helper["name"], languages[0]["raw"]["lanname"]);
        }
        if class == "1213" {
            let database = &identity::rows(self.structure.snapshot, "database")[0];
            assert_eq!(
                database["tablespace"]["oid"], id,
                "captured current tablespace witness"
            );
            assert_eq!(helper["type"], "tablespace");
            assert!(helper["schema"].is_null());
            assert_eq!(helper["name"], database["tablespace"]["spcname"]);
        }
        if class == "1260" {
            let roles: Vec<_> = identity::rows(self.structure.snapshot, "roles")
                .iter()
                .filter(|role| role["oid"] == id)
                .collect();
            assert_eq!(roles.len(), 1, "global role endpoint witness");
            super::bindings::role(helper, roles[0]);
        } else {
            assert!(
                id.parse::<u32>().unwrap() < 16384,
                "uncaptured nonnative local endpoint"
            );
        }
        let key = super::bindings::raw_endpoint(class, id, sub, helper, address);
        (key, helper.clone(), address.cloned())
    }
}
