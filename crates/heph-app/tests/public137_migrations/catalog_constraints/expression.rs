//! Restricted, lossless node-token handling for the finite CHECK declarations.
//! Only an actual FUNCEXPR's typed funcid may resolve to either allowed helper.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(in super::super) struct Token<'a> {
    pub(in super::super) start: usize,
    pub(in super::super) end: usize,
    pub(in super::super) value: &'a str,
}

struct Node<'a> {
    tag: &'a str,
    fields: BTreeSet<&'a str>,
}

pub(in super::super) const FUNCTION_FIELDS: &[&str] = &[
    ":funcid",
    ":funcresulttype",
    ":funcretset",
    ":funcvariadic",
    ":funcformat",
    ":funccollid",
    ":inputcollid",
    ":args",
    ":location",
];

pub(super) fn normalized(text: &str, helpers: &BTreeMap<String, Value>) -> Value {
    let tokens = tokens(text);
    assert_eq!(tokens.first().map(|token| token.value), Some("{"));
    assert_eq!(tokens.last().map(|token| token.value), Some("}"));
    let mut nodes = Vec::new();
    let mut delimiters = Vec::new();
    let mut parts = Vec::new();
    let mut preserved = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token.value {
            "{" => {
                delimiters.push("}");
                let tag = tokens.get(index + 1).expect("node tag").value;
                assert!(
                    !tag.is_empty()
                        && tag.bytes().all(|byte| byte.is_ascii_uppercase()
                            || byte.is_ascii_digit()
                            || byte == b'_')
                );
                nodes.push(Node {
                    tag,
                    fields: BTreeSet::new(),
                });
            }
            "}" => {
                assert_eq!(delimiters.pop(), Some("}"));
                let node = nodes.pop().expect("node opening");
                if node.tag == "FUNCEXPR" {
                    assert_eq!(node.fields, FUNCTION_FIELDS.iter().copied().collect());
                }
            }
            "(" => delimiters.push(")"),
            ")" => assert_eq!(delimiters.pop(), Some(")")),
            field if field.starts_with(':') => {
                assert_eq!(
                    delimiters.last(),
                    Some(&"}"),
                    "field directly belongs to node"
                );
                let node = nodes.last_mut().expect("field belongs to node");
                assert!(node.fields.insert(field), "duplicate node field");
                if matches!(
                    field,
                    ":funcid"
                        | ":opfuncid"
                        | ":hashfuncid"
                        | ":negfuncid"
                        | ":aggfnoid"
                        | ":winfnoid"
                ) {
                    let datum = tokens.get(index + 1).expect("typed function OID");
                    assert!(datum.value.bytes().all(|byte| byte.is_ascii_digit()));
                    let number: u32 = datum.value.parse().expect("function OID fits u32");
                    if field == ":funcid" {
                        assert_eq!(node.tag, "FUNCEXPR");
                        assert!(number > 0);
                    }
                    if number >= 16384 {
                        assert_eq!(node.tag, "FUNCEXPR", "unsupported user function node");
                        assert_eq!(field, ":funcid", "unsupported user function field");
                        let binding = helpers
                            .get(datum.value)
                            .expect("unsupported user CHECK function");
                        parts.push(json!(&text[preserved..datum.start]));
                        parts.push(json!({"FUNCEXPR.funcid": binding}));
                        preserved = datum.end;
                    }
                }
            }
            _ => {}
        }
    }
    assert!(nodes.is_empty() && delimiters.is_empty());
    parts.push(json!(&text[preserved..]));
    Value::Array(parts)
}

pub(in super::super) fn tokens(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let start = at;
        if b"{}()".contains(&bytes[at]) {
            at += 1;
        } else if bytes[at] == b'"' {
            at += 1;
            loop {
                let byte = *bytes.get(at).expect("closed node string");
                at += 1;
                if byte == b'\\' {
                    bytes.get(at).expect("complete node escape");
                    at += 1;
                } else if byte == b'"' {
                    break;
                }
            }
        } else {
            while at < bytes.len()
                && !bytes[at].is_ascii_whitespace()
                && !b"{}()".contains(&bytes[at])
            {
                if bytes[at] == b'\\' {
                    at += 1;
                    bytes.get(at).expect("complete node token escape");
                }
                at += 1;
            }
        }
        result.push(Token {
            start,
            end: at,
            value: &text[start..at],
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::normalized;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    const NODE: &str = "{FUNCEXPR :funcid 20000 :funcresulttype 16 :funcretset false :funcvariadic false :funcformat 0 :funccollid 0 :inputcollid 0 :args ({CONST :constvalue \"20000 :funcid 20000 {FUNCEXPR}\"}) :location -1}";

    #[test]
    fn only_typed_funcid_changes() {
        let binding =
            json!({"signature": "public.private_volume_guest_path_is_valid(pg_catalog.text)"});
        let helpers = BTreeMap::from([("20000".into(), binding.clone())]);
        let result = normalized(NODE, &helpers);
        assert_eq!(result[0], "{FUNCEXPR :funcid ");
        assert_eq!(result[1], json!({"FUNCEXPR.funcid": binding}));
        assert_eq!(result[2], &NODE[23..]);
    }

    #[test]
    fn builtins_remain_byte_exact() {
        let text = NODE.replacen(":funcid 20000", ":funcid 1250", 1);
        assert_eq!(normalized(&text, &BTreeMap::new()), json!([text]));
    }

    #[test]
    #[should_panic(expected = "unsupported user CHECK function")]
    fn unknown_helper_is_denied() {
        normalized(NODE, &BTreeMap::new());
    }

    #[test]
    #[should_panic(expected = "unsupported user function node")]
    fn another_node_cannot_normalize_user_ids() {
        normalized(
            "{OPEXPR :opfuncid 20000}",
            &BTreeMap::from([("20000".into(), json!("helper"))]),
        );
    }

    fn helpers() -> BTreeMap<String, Value> {
        BTreeMap::from([
            (
                "20000".into(),
                json!({"signature": "public.private_volume_guest_path_is_valid(pg_catalog.text)"}),
            ),
            (
                "20001".into(),
                json!({"signature": "public.run_cleanup_identifier_is_valid(pg_catalog.text)"}),
            ),
        ])
    }

    #[test]
    fn nested_repeated_helpers_preserve_all_other_bytes() {
        let inner = NODE.replacen(":funcid 20000", ":funcid 20001", 1);
        let text = NODE.replacen(":args (", &format!(":args ({inner} {inner} "), 1);
        let helpers = helpers();
        let result = normalized(&text, &helpers);
        let mut rebuilt = String::new();
        let mut seen = Vec::new();
        for part in result.as_array().unwrap() {
            if let Some(fragment) = part.as_str() {
                rebuilt.push_str(fragment);
            } else {
                let binding = &part["FUNCEXPR.funcid"];
                let id = if binding == &helpers["20000"] {
                    "20000"
                } else {
                    assert_eq!(binding, &helpers["20001"]);
                    "20001"
                };
                seen.push(id);
                rebuilt.push_str(id);
            }
        }
        assert_eq!(seen, ["20000", "20001", "20001"]);
        assert_eq!(rebuilt, text);
    }

    #[test]
    #[should_panic(expected = "duplicate node field")]
    fn duplicate_funcid_is_denied() {
        let text = NODE.replacen(":funcid 20000", ":funcid 20000 :funcid 20000", 1);
        normalized(&text, &helpers());
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed")]
    fn missing_function_field_is_denied() {
        let text = NODE.replacen(" :funcresulttype 16", "", 1);
        normalized(&text, &helpers());
    }

    #[test]
    fn escaped_unquoted_tokens_and_quotes_are_literal_data() {
        let text =
            r#"{CONST :constvalue \:funcid\ 20000 \{FUNCEXPR\} ":funcid 20000 \"{FUNCEXPR}\""}"#;
        assert_eq!(normalized(text, &helpers()), json!([text]));
    }

    #[test]
    fn other_high_oids_casts_and_const_bytes_remain_exact() {
        let cast = "{COERCEVIAIO :arg {CONST :consttype 90000 :constvalue 8 [ 32 78 0 0 0 0 0 0 ]} :resulttype 91000 :resultcollid 92000 :coerceformat 1 :location -1}";
        let text = NODE
            .replacen(":funcresulttype 16", ":funcresulttype 70000", 1)
            .replacen(":funccollid 0", ":funccollid 80000", 1)
            .replacen(":inputcollid 0", ":inputcollid 81000", 1)
            .replacen(":args (", &format!(":args ({cast} "), 1);
        let helpers = helpers();
        let result = normalized(&text, &helpers);
        assert_eq!(result.as_array().unwrap().len(), 3);
        assert_eq!(result[0], "{FUNCEXPR :funcid ");
        assert_eq!(result[1], json!({"FUNCEXPR.funcid": helpers["20000"]}));
        assert_eq!(result[2], &text[23..]);
    }
}
