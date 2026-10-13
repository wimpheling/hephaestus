//! Restricted SQL token boundaries; comments, quoted data and bodies are opaque.
fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut at = start + 1;
    loop {
        let byte = *bytes.get(at).expect("closed SQL quote");
        at += 1;
        if byte == quote {
            if bytes.get(at) == Some(&quote) {
                at += 1;
            } else {
                return at;
            }
        } else if byte == b'\\' && quote == b'\'' {
            bytes.get(at).expect("complete SQL escape");
            at += 1;
        }
    }
}

fn comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1;
    let mut at = start + 2;
    while depth > 0 {
        assert!(at + 1 < bytes.len(), "closed SQL comment");
        if bytes[at..].starts_with(b"/*") {
            depth += 1;
            at += 2;
        } else if bytes[at..].starts_with(b"*/") {
            depth -= 1;
            at += 2;
        } else {
            at += 1;
        }
    }
    at
}

pub(super) fn tokens(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut result = Vec::new();
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"--") {
            at += bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .unwrap_or(bytes.len() - at);
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at = comment_end(bytes, at);
            continue;
        }
        if matches!(bytes[at], b'\'' | b'"') {
            at = quoted_end(bytes, at, bytes[at]);
            result.push("<quoted>".into());
            continue;
        }
        if bytes[at] == b'$' {
            let length = bytes[at + 1..]
                .iter()
                .position(|byte| *byte == b'$')
                .expect("dollar delimiter")
                + 2;
            let delimiter = &sql[at..at + length];
            assert!(
                delimiter[1..delimiter.len() - 1]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            );
            let body = at + length;
            at = body + sql[body..].find(delimiter).expect("closed dollar body") + length;
            result.push("<body>".into());
            continue;
        }
        let start = at;
        if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
            at += 1;
            while bytes
                .get(at)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                at += 1;
            }
        } else {
            at += sql[at..].chars().next().unwrap().len_utf8();
        }
        result.push(sql[start..at].to_ascii_lowercase());
    }
    result
}
