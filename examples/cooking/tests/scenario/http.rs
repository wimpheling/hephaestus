// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn read_http_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Vec<u8> {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 2048];
        let length = stream
            .read(&mut chunk)
            .await
            .expect("read bounded HTTP request");
        assert_ne!(length, 0);
        request.extend_from_slice(&chunk[..length]);
        assert!(request.len() <= 32768);
        if let Some(separator) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..separator]).expect("headers");
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .expect("content length")
                .parse()
                .expect("bounded length");
            if request.len() == separator + 4 + length {
                return request;
            }
        }
    }
}

pub(crate) async fn invoke_relay(
    body: serde_json::Value,
    database: &Path,
    credential: &str,
) -> serde_json::Value {
    let script = "import importlib.util,json,sys\nspec=importlib.util.spec_from_file_location('relay',sys.argv[1]); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)\nrequest=json.load(sys.stdin); relay=module.Relay(sys.argv[2],request['credential'].encode()); status,body=relay.deliver('Bearer '+request['credential'],json.dumps(request['body']).encode()); assert status==200; print(json.dumps(body))";
    let mut child = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(source_root().join("telegram-relay/relay.py"))
        .arg(database)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("external relay application");
    child
        .stdin
        .take()
        .expect("relay input")
        .write_all(
            &serde_json::to_vec(&serde_json::json!({"credential":credential,"body":body}))
                .expect("relay input JSON"),
        )
        .await
        .expect("relay input");
    let output = child
        .wait_with_output()
        .await
        .expect("relay application outcome");
    assert!(
        output.status.success(),
        "external relay rejected bounded request"
    );
    serde_json::from_slice(&output.stdout).expect("relay redacted response")
}

pub(crate) const fn expected_relay_ledger(update_slice: bool, crash_mode: bool) -> &'static str {
    if crash_mode {
        "['recipe-51','recipe-52','recipe-53','recipe-54','recipe-55']"
    } else if update_slice {
        "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46','recipe-47','recipe-48','recipe-49']"
    } else {
        "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46']"
    }
}

pub(crate) async fn assert_relay_ledger(database: &Path, update_slice: bool, crash_mode: bool) {
    let expected = expected_relay_ledger(update_slice, crash_mode);
    let script = format!(
        "import sqlite3,sys\nrows=sqlite3.connect(sys.argv[1]).execute('SELECT key FROM deliveries ORDER BY key').fetchall()\nassert [row[0] for row in rows] == {expected}, rows"
    );
    let output = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(database)
        .output()
        .await
        .expect("relay ledger census");
    assert!(
        output.status.success(),
        "relay ledger must retain exactly one row per logical recipe"
    );
}
