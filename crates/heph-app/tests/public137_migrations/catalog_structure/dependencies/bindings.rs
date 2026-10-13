//! Exact endpoint witnesses; generated names change only after typed joins.
use super::{address, identity, json, signed};
use serde_json::Value;

pub(super) struct Binding {
    pub key: String,
    pub helper: Value,
    pub canonical: Value,
    pub address: Option<Value>,
    pub canonical_address: Option<Value>,
}

impl Binding {
    pub fn new(key: String, helper: Value, names: &[String]) -> Self {
        identity::object(&helper);
        let address = json!({"type":helper["type"],"object_names":names,"object_args":[]});
        Self {
            key,
            canonical: helper.clone(),
            helper,
            canonical_address: Some(address.clone()),
            address: Some(address),
        }
    }

    pub fn check(&self, helper: &Value, actual_address: Option<&Value>) {
        assert_eq!(*helper, self.helper, "captured typed endpoint witness");
        if let Some(actual) = actual_address {
            address(actual, helper);
            assert_eq!(
                Some(actual),
                self.address.as_ref(),
                "complete endpoint address witness"
            );
        }
    }
}

pub(super) fn role(helper: &Value, raw: &Value) {
    identity::object(helper);
    assert_eq!(helper["type"], "role");
    assert!(helper["schema"].is_null());
    assert_eq!(helper["name"], raw["rolname"]);
}

pub(super) fn raw_endpoint(
    class: &str,
    id: &str,
    sub: i32,
    helper: &Value,
    actual_address: Option<&Value>,
) -> String {
    assert_ne!(id, "0");
    identity::object(helper);
    assert!(
        class == "1260" || id.parse::<u32>().unwrap() < 16384,
        "uncaptured nonnative local endpoint"
    );
    assert!(
        matches!(
            class,
            "1259"
                | "1247"
                | "1255"
                | "2606"
                | "2620"
                | "3256"
                | "2604"
                | "2618"
                | "3079"
                | "2615"
                | "2612"
                | "1260"
                | "1213"
                | "2616"
                | "2617"
                | "2753"
                | "2600"
                | "2601"
                | "2602"
                | "2603"
                | "2607"
                | "2605"
                | "3456"
                | "3600"
                | "3601"
                | "3602"
                | "3603"
                | "3764"
                | "826"
                | "1417"
                | "2328"
                | "1418"
                | "3381"
                | "3596"
                | "6104"
                | "6106"
        ),
        "known native endpoint catalog: class={class}, id={id}, sub={sub}"
    );
    if class != "1259" {
        assert_eq!(sub, 0, "nonrelation subobject");
    }
    if let Some(actual) = actual_address {
        address(actual, helper);
    }
    format!("literal({class},{id},{sub})")
}

pub(super) fn subobject(raw: &Value, shared: bool, reference: bool) -> i32 {
    if shared && reference {
        0
    } else {
        signed(&raw[if reference { "refobjsubid" } else { "objsubid" }])
    }
}
