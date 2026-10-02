//! Ordinary-user native probe; never installed at the privileged builder path.

use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::MetadataExt,
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const STATE: &str = "/var/lib/hephaestus/state.db";
const DATA: &str = "/data/workload.db";

pub fn run() -> Result<()> {
    let mode = std::env::args().nth(2).ok_or("native probe mode missing")?;
    if !matches!(mode.as_str(), "fresh" | "reopen" | "seeded") {
        return Err("invalid native probe mode".into());
    }
    if rustix::process::geteuid().as_raw() != 10_001
        || rustix::process::getegid().as_raw() != 10_001
    {
        return Err("native probe must use ordinary guest UID/GID 10001".into());
    }
    verify_mount("/var/lib/hephaestus", false)?;
    verify_mount("/data", false)?;
    verify_mount("/facts", true)?;
    verify_read_only()?;
    if Path::new("/data/state.db").exists() {
        return Err("None initialization created a built-in data database".into());
    }
    verify_state(&mode)?;
    verify_data(&mode)?;
    println!(
        "GUEST_INITIALIZATION=1 mode={mode} uid=10001 gid=10001 ext4_mounts=1 device_identity=1 kernel_ro=1 ro_file_denied=1 none_no_builtin_db=1 sqlite_integrity=1"
    );
    Ok(())
}

fn database(path: &str, read_only: bool) -> Result<Connection> {
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    Ok(Connection::open_with_flags(
        path,
        flags | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?)
}

fn verify_state(mode: &str) -> Result<()> {
    if !Path::new(STATE).is_file() {
        return Err("typed built-in purpose did not create state.db".into());
    }
    let mut connection = database(STATE, mode != "fresh")?;
    let journal: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    let expected = if mode == "seeded" { "delete" } else { "wal" };
    if journal != expected {
        return Err(format!("state journal mode changed: {journal}").into());
    }
    if mode == "fresh" {
        let tables: i64 = connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |row| row.get(0),
        )?;
        if tables != 0 {
            return Err("initializer invented an application schema".into());
        }
        let transaction = connection.transaction()?;
        transaction.execute("CREATE TABLE application_owned(value TEXT NOT NULL)", [])?;
        transaction.execute(
            "INSERT INTO application_owned(value) VALUES (?1)",
            ["state persisted"],
        )?;
        transaction.commit()?;
    }
    let value: String = if mode == "seeded" {
        connection.query_row("SELECT value FROM preexisting_schema", [], |row| row.get(0))?
    } else {
        connection.query_row("SELECT value FROM application_owned", [], |row| row.get(0))?
    };
    if value
        != if mode == "seeded" {
            "seed preserved"
        } else {
            "state persisted"
        }
    {
        return Err("built-in state content was not retained".into());
    }
    integrity(&connection)?;
    println!("BUILTIN_STATE=1 mode={mode} journal={journal} schema_data_preserved=1");
    Ok(())
}

fn verify_data(mode: &str) -> Result<()> {
    let mut connection = if mode == "fresh" {
        if Path::new(DATA).exists() {
            return Err("named None workload DB already existed before app initialization".into());
        }
        let connection = Connection::open_with_flags(
            DATA,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")?;
        connection
    } else {
        database(DATA, false)?
    };
    if mode == "fresh" {
        let transaction = connection.transaction()?;
        transaction.execute("CREATE TABLE workload_owned(value TEXT NOT NULL)", [])?;
        transaction.execute("INSERT INTO workload_owned VALUES (?1)", ["data persisted"])?;
        transaction.commit()?;
        fs::write("/data/plain", "ordinary user persisted\n")?;
    }
    let value: String =
        connection.query_row("SELECT value FROM workload_owned", [], |row| row.get(0))?;
    if value != "data persisted"
        || fs::read_to_string("/data/plain")? != "ordinary user persisted\n"
    {
        return Err("ordinary named workload data did not survive reopen".into());
    }
    integrity(&connection)?;
    println!("NAMED_NONE=1 mode={mode} app_owned_sqlite=1 write_reopen=1");
    Ok(())
}

fn integrity(connection: &Connection) -> Result<()> {
    let check: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err("SQLite integrity check failed".into());
    }
    Ok(())
}

fn verify_read_only() -> Result<()> {
    let path = "/facts/sentinel";
    let metadata = fs::metadata(path)?;
    if metadata.uid() != 10_001 || metadata.gid() != 10_001 || metadata.mode() & 0o200 == 0 {
        return Err("RO fixture must give ordinary guest owner write permission".into());
    }
    if fs::read_to_string(path)? != "read-only ordinary user\n" {
        return Err("RO sentinel differs".into());
    }
    match OpenOptions::new().write(true).open(path) {
        Err(error) if error.raw_os_error() == Some(libc::EROFS) => Ok(()),
        Err(error) => Err(format!("expected filesystem RO denial, got {error}").into()),
        Ok(_) => Err("RO mount accepted ordinary-user write access".into()),
    }
}

fn verify_mount(path: &str, read_only: bool) -> Result<()> {
    let details = fs::read_to_string("/proc/self/mountinfo")?;
    let entry = details
        .lines()
        .find(|line| line.split_whitespace().nth(4) == Some(path))
        .ok_or("selected guest mount missing")?;
    let (mount, filesystem) = entry.split_once(" - ").ok_or("invalid mountinfo")?;
    if filesystem.split_whitespace().next() != Some("ext4")
        || mount
            .split_whitespace()
            .nth(5)
            .ok_or("missing mount flags")?
            .split(',')
            .any(|flag| flag == "ro")
            != read_only
    {
        return Err("selected filesystem type or access mode differs".into());
    }
    let device = fs::metadata(path)?.dev();
    for block in fs::read_dir("/sys/class/block")? {
        let block = block?;
        let node = Path::new("/dev").join(block.file_name());
        if fs::metadata(node)?.rdev() == device {
            let mode = fs::read_to_string(block.path().join("ro"))?;
            if mode.trim() != if read_only { "1" } else { "0" } {
                return Err("kernel block access differs from guest mount".into());
            }
            return Ok(());
        }
    }
    Err("mounted filesystem has no matching kernel block device".into())
}
