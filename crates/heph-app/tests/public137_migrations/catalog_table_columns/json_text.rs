//! Lossless slices of the fixed, sealed `PostgreSQL` JSONB capture. In particular,
//! missing datums never round-trip through `serde_json`'s numeric representation.
use std::collections::BTreeSet;

pub fn array_items(text: &str) -> Vec<&str> {
    parts(text, b'[', b']')
}

pub fn field<'a>(text: &'a str, wanted: &str) -> &'a str {
    let mut found = None;
    let mut names = BTreeSet::new();
    for entry in parts(text, b'{', b'}') {
        let mut quoted = false;
        let mut escaped = false;
        let mut separator = None;
        for (index, byte) in entry.bytes().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if byte == b':' {
                separator = Some(index);
                break;
            }
        }
        let separator = separator.expect("captured object field needs a colon");
        let name: String = serde_json::from_str(entry[..separator].trim()).unwrap();
        assert!(names.insert(name.clone()), "duplicate captured JSON field");
        let value = entry[separator + 1..].trim();
        assert!(!value.is_empty());
        if name == wanted {
            assert!(found.replace(value).is_none());
        }
    }
    found.expect("missing required captured JSON field")
}

fn parts(text: &str, opening: u8, closing: u8) -> Vec<&str> {
    let text = text.trim();
    assert_eq!(text.as_bytes().first(), Some(&opening));
    assert_eq!(text.as_bytes().last(), Some(&closing));
    let inner = &text[1..text.len() - 1];
    let mut parts = Vec::new();
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, byte) in inner.bytes().enumerate() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => assert_eq!(stack.pop(), Some(byte)),
            b',' if stack.is_empty() => {
                let part = inner[start..index].trim();
                assert!(!part.is_empty());
                parts.push(part);
                start = index + 1;
            }
            _ => {}
        }
    }
    assert!(!quoted && !escaped && stack.is_empty());
    let tail = inner[start..].trim();
    if tail.is_empty() {
        assert!(parts.is_empty());
    } else {
        parts.push(tail);
    }
    parts
}
