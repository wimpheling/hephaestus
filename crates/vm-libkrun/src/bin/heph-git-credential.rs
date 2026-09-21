//! Standard Git credential helper for the exact runtime-Git bootstrap.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use zeroize::Zeroizing;

const USERNAME: &str = "heph-runtime";
const PREFIX: &str = "heph_git_v1_";
const HOST_ENV: &str = "HEPH_RUNTIME_GIT_HOST";
const PATH_ENV: &str = "HEPH_RUNTIME_GIT_PATH";
const AUTHORITY_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";

#[derive(Deserialize)]
struct Authority {
    runtime_git_credential: Option<SensitiveString>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct SensitiveString(String);

impl Drop for SensitiveString {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let action = std::env::args().nth(1).ok_or_else(|| invalid("action"))?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    if action != "get" {
        return Ok(());
    }
    let fields = input
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected_host = std::env::var(HOST_ENV).map_err(|_| invalid("expected host"))?;
    let expected_path = std::env::var(PATH_ENV).map_err(|_| invalid("expected path"))?;
    let actual_path = fields
        .get("path")
        .map(|path| path.trim_start_matches('/'))
        .ok_or_else(|| invalid("credential path"))?;
    if fields.get("protocol") != Some(&"http")
        || fields.get("host") != Some(&expected_host.as_str())
        || actual_path != expected_path
    {
        return Err(invalid("credential target"));
    }
    let path = std::env::var(AUTHORITY_ENV).map_err(|_| invalid("authority path"))?;
    let metadata = fs::symlink_metadata(&path).map_err(|_| invalid("authority file"))?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o777 != 0o400
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(invalid("authority file protection"));
    }
    let bytes = Zeroizing::new(fs::read(Path::new(&path)).map_err(|_| invalid("authority file"))?);
    let mut authority: Authority =
        serde_json::from_slice(&bytes).map_err(|_| invalid("authority"))?;
    let raw = authority
        .runtime_git_credential
        .take()
        .ok_or_else(|| invalid("runtime Git credential"))?;
    let token = canonical_token(&raw.0)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(stdout, "username={USERNAME}")?;
    writeln!(stdout, "password={}", token.as_str())?;
    Ok(())
}

fn canonical_token(raw: &str) -> io::Result<Zeroizing<String>> {
    let mut decoded = Zeroizing::new([0_u8; 32]);
    if raw.len() != 64 {
        return Err(invalid("runtime Git credential length"));
    }
    for (index, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    let mut token = Zeroizing::new(String::from(PREFIX));
    token.push_str(&URL_SAFE_NO_PAD.encode(decoded.as_slice()));
    Ok(token)
}

fn hex(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid("runtime Git credential encoding")),
    }
}

fn invalid(field: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, field)
}

#[cfg(test)]
mod tests {
    use super::canonical_token;

    #[test]
    fn canonical_token_uses_runtime_git_prefix_and_url_alphabet() {
        let token = canonical_token(&"5c".repeat(32)).expect("canonical token");
        assert!(token.starts_with("heph_git_v1_"));
        assert!(!token.contains('='));
        assert!(!token.contains('+'));
        assert!(!token.contains('/'));
    }

    #[test]
    fn malformed_credential_is_rejected_without_echoing_secret() {
        let error = canonical_token("not-a-secret").expect_err("malformed token");
        assert!(!error.to_string().contains("not-a-secret"));
    }
}
