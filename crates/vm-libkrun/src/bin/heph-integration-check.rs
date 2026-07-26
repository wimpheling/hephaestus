//! Guest-side hardware integration probe for the libkrun backend.

use rusqlite::Connection;
use std::{
    ffi::CString,
    fs,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    os::unix::ffi::OsStrExt,
    path::Path,
    time::Duration,
};

fn main() {
    if let Err(error) = run() {
        let _write_result = writeln!(io::stderr(), "integration-check: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("--serve-http") => return serve_http().map_err(Into::into),
        Some("--expect-network-disabled") => return expect_network_disabled(),
        Some("--ignore-cancellation") => return ignore_cancellation(),
        Some(argument) => {
            return Err(format!("unknown integration-check argument: {argument}").into());
        }
        None => {}
    }

    eprintln!("stderr=ok");
    verify_disk()?;
    println!("sqlite=ok");
    verify_mounts()?;
    println!("mounts=ok");
    let resolved = ("example.com", 80)
        .to_socket_addrs()?
        .next()
        .ok_or("DNS returned no addresses")?;
    println!("dns=ok");
    verify_tcp(resolved)?;
    println!("tcp=ok");
    verify_udp_dns()?;
    println!("udp=ok");
    Ok(())
}

fn verify_disk() -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all("/sqlite")?;
    mount_disk(Path::new("/dev/vda"), Path::new("/sqlite"))?;
    let database = Path::new("/sqlite/agent.db");
    let connection = Connection::open(database)?;
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
    unmount_disk(Path::new("/sqlite"))?;
    Ok(())
}

// The integration probe is PID 2 inside an isolated guest and must mount the
// prepared block device without depending on a distribution mount helper.
#[allow(unsafe_code)]
fn mount_disk(source: &Path, target: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "disk path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: all pointers refer to live NUL-terminated strings, the optional
    // data pointer is null, and mount flags contain only Linux constants.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            c"ext4".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV,
            std::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// libkrun exits when the guest command finishes rather than performing a full
// guest shutdown. Explicitly unmounting makes the persistence assertion test
// completed block I/O instead of Linux's in-guest page cache.
#[allow(unsafe_code)]
fn unmount_disk(target: &Path) -> io::Result<()> {
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: `target` is a live NUL-terminated string and no flags are used.
    let result = unsafe { libc::umount2(target.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn verify_mounts() -> Result<(), Box<dyn std::error::Error>> {
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

fn serve_http() -> io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", 8080))?;
    println!("http=ready");
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut request = [0_u8; 1024];
    let read = stream.read(&mut request)?;
    if read == 0 || !request[..read].starts_with(b"GET / HTTP/1.") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request",
        ));
    }
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")?;
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    // Keep the VMM alive briefly so passt can drain its socket before the
    // command's final exit tears down virtio-net.
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

fn expect_network_disabled() -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = "1.1.1.1:80".parse()?;
    if TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok() {
        return Err("disabled network unexpectedly allowed outbound TCP".into());
    }
    println!("network-disabled=ok");
    Ok(())
}

#[allow(unsafe_code)]
fn ignore_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: installing SIG_IGN for SIGTERM changes only this isolated guest
    // test process and does not dereference memory.
    let previous = unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    if previous == libc::SIG_ERR {
        return Err(io::Error::last_os_error().into());
    }
    println!("ignore-cancellation=ready");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn verify_tcp(address: SocketAddr) -> io::Result<()> {
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

fn verify_udp_dns() -> Result<(), Box<dyn std::error::Error>> {
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
