use rusqlite::Connection;
use std::{
    fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, UdpSocket},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    time::Duration,
};

pub fn verify_disk() -> Result<(), Box<dyn std::error::Error>> {
    let database = Path::new("/var/lib/hephaestus/state.db");
    let connection = Connection::open(database)?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err("SQLite state database is not in WAL mode".into());
    }
    let table_count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'probe'",
        [],
        |row| row.get(0),
    )?;
    let previous_rows: i64 = if table_count == 0 {
        0
    } else {
        connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?
    };
    println!("sqlite_previous={previous_rows}");
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS probe(value TEXT);
         INSERT INTO probe VALUES('ok');",
    )?;
    let value: String = connection.query_row(
        "SELECT value FROM probe ORDER BY rowid DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    if value != "ok" {
        return Err("SQLite readback failed".into());
    }
    drop(connection);
    Ok(())
}

pub fn verify_sqlite_rollback() -> Result<(), Box<dyn std::error::Error>> {
    let database = Path::new("/var/lib/hephaestus/state.db");
    let mut connection = Connection::open(database)?;
    let before: i64 = connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT INTO probe VALUES('must-rollback')", [])?;
    transaction.rollback()?;
    let after: i64 = connection.query_row("SELECT count(*) FROM probe", [], |row| row.get(0))?;
    if after != before {
        return Err("agent-owned SQLite rollback retained a mutation".into());
    }
    Ok(())
}

pub fn verify_mounts() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::current_dir()? != Path::new("/workspace") {
        return Err("requested working directory was not applied".into());
    }
    if fs::read_to_string("/repository/integration-marker")?.trim() != "repository" {
        return Err("read-only repository marker is invalid".into());
    }
    match fs::write("/repository/write-must-fail", "invalid") {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) => {}
        Err(error) => return Err(format!("unexpected read-only mount error: {error}").into()),
        Ok(()) => return Err("read-only repository accepted a write".into()),
    }
    let workspace_marker = Path::new("/workspace/integration-marker");
    fs::write(workspace_marker, "workspace")?;
    if fs::read_to_string(workspace_marker)? != "workspace" {
        return Err("writable workspace readback failed".into());
    }
    Ok(())
}

pub fn verify_secrets() -> Result<(), Box<dyn std::error::Error>> {
    const SENTINEL: &str = "libkrun-secret-sentinel-8a4c";
    let directory = Path::new("/run/hephaestus-secrets");
    let secret = directory.join("model");
    let directory_metadata = fs::symlink_metadata(directory)?;
    let metadata = fs::symlink_metadata(&secret)?;
    if !directory_metadata.is_dir() || !metadata.is_file() {
        return Err("raw secret mount contains an unexpected object".into());
    }
    if directory_metadata.permissions().mode() & 0o777 != 0o500
        || metadata.permissions().mode() & 0o777 != 0o400
    {
        return Err("raw secret mount permissions are unsafe".into());
    }
    if metadata.uid() != 10_001 || metadata.gid() != 10_001 {
        return Err("raw secret ownership does not match the guest agent".into());
    }
    if fs::read_to_string(&secret)? != SENTINEL {
        return Err("raw secret contents do not match the exact slot".into());
    }
    for path in [&secret, &directory.join("write-must-fail")] {
        match fs::write(path, "invalid") {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
                ) => {}
            Err(error) => {
                return Err(format!("unexpected secret mount write error: {error}").into());
            }
            Ok(()) => return Err("read-only raw secret mount accepted a write".into()),
        }
    }
    for proc_file in ["/proc/self/environ", "/proc/self/cmdline"] {
        if fs::read(proc_file)?
            .windows(SENTINEL.len())
            .any(|window| window == SENTINEL.as_bytes())
        {
            return Err("raw secret leaked into process metadata".into());
        }
    }
    Ok(())
}

pub fn verify_tcp(address: SocketAddr) -> io::Result<()> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(b"HEAD / HTTP/1.0\r\nHost: example.com\r\n\r\n")?;
    let mut response = [0_u8; 16];
    let read = stream.read(&mut response)?;
    if read == 0 {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "TCP peer returned no response",
        ))
    } else {
        Ok(())
    }
}

pub fn verify_udp_dns() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = resolver_address()?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    let query = dns_query();
    socket.send_to(&query, resolver)?;
    let mut response = [0_u8; 512];
    let (length, _) = socket.recv_from(&mut response)?;
    if length < 12 || response[..2] != query[..2] || response[3] & 0x0f != 0 {
        return Err("invalid UDP DNS response".into());
    }
    Ok(())
}

fn resolver_address() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let resolv_conf = fs::read_to_string("/etc/resolv.conf")?;
    let address = resolv_conf
        .lines()
        .find_map(|line| line.strip_prefix("nameserver "))
        .ok_or("no DNS resolver is configured")?;
    Ok(format!("{}:53", address.trim()).parse()?)
}

fn dns_query() -> Vec<u8> {
    let mut query = vec![
        0x48, 0x50, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for label in ["example", "com"] {
        query.push(u8::try_from(label.len()).expect("DNS label length fits u8"));
        query.extend_from_slice(label.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    query
}
