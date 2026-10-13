//! Reuse the CHECK lexer; translate only four explicitly typed PG17 OID slots.
use super::super::super::catalog_constraints::expression::tokens;
use super::shapes;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

struct Node<'a> {
    tag: &'a str,
    fields: BTreeSet<String>,
    kind: Option<&'a str>,
}

fn binding<'a>(
    tag: &str,
    field: &str,
    value: &str,
    functions: &'a BTreeMap<String, Value>,
    classes: &'a BTreeMap<String, Value>,
) -> Option<&'a Value> {
    if field == ":funcid" {
        assert_eq!(tag, "FUNCEXPR");
        let id: u32 = value.parse().expect("typed function OID");
        assert_ne!(id, 0);
        return if id >= 16384 {
            Some(functions.get(value).expect("unqualified policy function"))
        } else {
            None
        };
    }
    if field == ":relid" || (tag == "TARGETENTRY" && field == ":resorigtbl") {
        assert!(
            matches!(
                (tag, field),
                ("RANGETBLENTRY" | "RTEPERMISSIONINFO", ":relid") | ("TARGETENTRY", ":resorigtbl")
            ),
            "typed class slot"
        );
        let id: u32 = value.parse().expect("typed relation OID");
        if id == 0 {
            assert_eq!(tag, "TARGETENTRY", "actual relation required");
            return None;
        }
        return Some(classes.get(value).expect("unqualified policy relation"));
    }
    if matches!(field, ":opfuncid" | ":hashfuncid" | ":negfuncid") {
        assert!(
            value.parse::<u32>().expect("function OID") < 16384,
            "unsupported nonbuiltin operator function"
        );
    }
    None
}

pub(super) fn normalized(
    text: &str,
    functions: &BTreeMap<String, Value>,
    classes: &BTreeMap<String, Value>,
) -> Value {
    let tokens = tokens(text);
    assert_eq!(tokens.first().map(|token| token.value), Some("{"));
    assert_eq!(tokens.last().map(|token| token.value), Some("}"));
    let mut nodes = Vec::<Node<'_>>::new();
    let mut delimiters = Vec::new();
    let mut parts = Vec::new();
    let mut preserved = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token.value {
            "{" => {
                delimiters.push("}");
                let tag = tokens.get(index + 1).expect("AST tag").value;
                nodes.push(Node {
                    tag,
                    fields: BTreeSet::new(),
                    kind: None,
                });
            }
            "}" => {
                assert_eq!(delimiters.pop(), Some("}"));
                let node = nodes.pop().expect("AST node");
                assert_eq!(
                    node.fields,
                    shapes::fields(node.tag, node.kind),
                    "complete known node shape"
                );
            }
            "(" => delimiters.push(")"),
            ")" => assert_eq!(delimiters.pop(), Some(")")),
            field if field.starts_with(':') => {
                assert_eq!(
                    delimiters.last(),
                    Some(&"}"),
                    "field directly belongs to node"
                );
                let node = nodes.last_mut().expect("field node");
                assert!(node.fields.insert(field.into()), "duplicate AST field");
                let datum = tokens.get(index + 1).expect("field datum");
                if node.tag == "RANGETBLENTRY" && field == ":rtekind" {
                    node.kind = Some(datum.value);
                }
                if let Some(value) = binding(node.tag, field, datum.value, functions, classes) {
                    parts.push(json!(&text[preserved..datum.start]));
                    parts.push(json!({"slot": format!("{}.{field}", node.tag), "binding": value}));
                    preserved = datum.end;
                }
            }
            _ => {}
        }
    }
    assert!(nodes.is_empty() && delimiters.is_empty());
    parts.push(json!(&text[preserved..]));
    Value::Array(parts)
}
