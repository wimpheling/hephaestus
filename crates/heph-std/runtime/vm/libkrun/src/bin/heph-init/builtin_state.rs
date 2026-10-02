//! Isolated preparation of the existing built-in database on a pinned mount.

#[path = "builtin_state/files.rs"]
mod files;

use super::volume_mounts::mounted_identity;
use rusqlite::{Connection, OpenFlags};
use rustix::fs::{Mode, OFlags, fsync, open, openat};
use std::{
    io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::Command,
};

const HELPER_ARGUMENT: &str = "--initialize-builtin-state";
const DATABASE: &str = "state.db";
const SIDECARS: [&str; 3] = ["state.db-wal", "state.db-shm", "state.db-journal"];

/// Executes in a child so the parent guest bootstrap never changes its cwd.
pub fn initialize_pinned(directory: &OwnedFd) -> io::Result<()> {
    let (device, mount) = mounted_identity(directory)?;
    let mut command = Command::new("/proc/self/exe");
    command.args([HELPER_ARGUMENT, &device.to_string(), &mount.to_string()]);
    pin_child_directory(&mut command, directory);
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(
            "built-in state initialization helper failed",
        ));
    }
    Ok(())
}

// The only pre-exec operation is an async-signal-safe syscall on an owned FD.
#[allow(unsafe_code)]
pub fn pin_child_directory(command: &mut Command, directory: &OwnedFd) {
    let descriptor = directory.as_raw_fd();
    // SAFETY: the caller retains the descriptor through spawn/status. No Rust
    // lock/allocation occurs after fork. CLOEXEC closes it after cwd is pinned.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(descriptor) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
}

/// Recognizes the private child entrypoint before any control/socket setup.
pub fn helper_entry() -> Option<io::Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) != Some(HELPER_ARGUMENT) {
        return None;
    }
    Some((|| {
        if args.len() != 4 {
            return Err(invalid("invalid initialization helper arguments"));
        }
        let expected = (
            args[2]
                .parse::<u64>()
                .map_err(|_| invalid("invalid mount device"))?,
            args[3]
                .parse::<u64>()
                .map_err(|_| invalid("invalid mount identity"))?,
        );
        let directory = open(
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        verify_mount(&directory, expected)?;
        prepare_database(&directory, crate::AGENT_UID, crate::AGENT_GID)
    })())
}

pub fn verify_mount(directory: &OwnedFd, expected: (u64, u64)) -> io::Result<()> {
    if mounted_identity(directory)? != expected {
        return Err(invalid(
            "built-in initialization mount differs from pinned target",
        ));
    }
    Ok(())
}

fn prepare_database(directory: &OwnedFd, uid: u32, gid: u32) -> io::Result<()> {
    let existing = files::regular_file(directory, DATABASE)?;
    let sidecars = SIDECARS
        .iter()
        .map(|name| files::regular_file(directory, name))
        .collect::<io::Result<Vec<_>>>()?;
    if existing.is_none() && sidecars.iter().any(Option::is_some) {
        return Err(invalid(
            "orphan built-in database sidecars require recovery",
        ));
    }
    drop(sidecars);
    let fresh = existing.is_none();
    let database = if let Some(database) = existing {
        if rustix::fs::fstat(&database)?.st_size == 0 {
            return Err(invalid(
                "existing empty built-in database requires recovery",
            ));
        }
        database
    } else {
        let database = openat(
            directory,
            DATABASE,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )?;
        fsync(&database)?;
        fsync(directory)?;
        database
    };
    // The helper cwd is the verified pinned mount. Relative SQLite NOFOLLOW
    // paths avoid magic-link ancestors, and never follow existing DB/sidecars.
    let connection = Connection::open_with_flags(
        DATABASE,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(io::Error::other)?;
    if fresh {
        let mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(io::Error::other)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(invalid("SQLite refused WAL mode"));
        }
        connection
            .execute_batch("PRAGMA synchronous = FULL;")
            .map_err(io::Error::other)?;
    } else {
        // Existing application schemas and journal mode are never initialized
        // again. SQLite performs its normal recovery and checks committed data.
        let integrity: String = connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(io::Error::other)?;
        if integrity != "ok" {
            return Err(invalid("existing built-in database is inconsistent"));
        }
    }
    drop(connection);
    files::give_to_agent(&database, uid, gid)?;
    fsync(&database)?;
    drop(database);
    for name in SIDECARS {
        if let Some(file) = files::regular_file(directory, name)? {
            files::give_to_agent(&file, uid, gid)?;
            fsync(&file)?;
        }
    }
    files::give_to_agent(directory, uid, gid)?;
    fsync(directory)?;
    Ok(())
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[cfg(test)]
#[path = "builtin_state/tests.rs"]
mod tests;
