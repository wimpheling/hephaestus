use rusqlite::Connection;
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    os::unix::ffi::OsStrExt,
    os::unix::fs::chown,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vm_libkrun::protocol::{GuestMessage, GuestStateVolume};

use crate::{AGENT_GID, AGENT_UID, CONTROL_CONNECT_TIMEOUT, vsock, write_frame};

pub fn mount_state_volume(volume: &GuestStateVolume) -> io::Result<PathBuf> {
    let filesystem_uuid = Uuid::parse_str(&volume.filesystem_uuid)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if !volume.guest_path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state-volume mount path must be absolute",
        ));
    }
    let device = find_ext4_device(filesystem_uuid)?;
    fs::create_dir_all(&volume.guest_path)?;
    mount_ext4(&device, &volume.guest_path)?;
    if volume.guest_path == Path::new("/var/lib/hephaestus") {
        initialize_database(&volume.guest_path)?;
    }
    Ok(volume.guest_path.clone())
}

/// Sends a bounded bootstrap diagnostic while the host control stream is
/// still available. This keeps mount and command failures distinguishable
/// from a worker that disappears before guest initialization completes.
pub fn send_guest_error(control: &mut File, code: &str, error: &impl std::fmt::Display) {
    let mut message = error.to_string();
    message.truncate(1_024);
    let _write_result = write_frame(
        control,
        &GuestMessage::Error {
            code: code.to_owned(),
            message,
        },
    );
}

fn find_ext4_device(expected: Uuid) -> io::Result<PathBuf> {
    find_ext4_device_in(expected, Path::new("/sys/class/block"), Path::new("/dev"))
}

pub fn find_ext4_device_in(
    expected: Uuid,
    block_root: &Path,
    device_root: &Path,
) -> io::Result<PathBuf> {
    for entry in fs::read_dir(block_root)? {
        let name = entry?.file_name();
        let device = device_root.join(name);
        let Ok(mut file) = File::open(&device) else {
            continue;
        };
        let mut superblock = [0_u8; 120];
        if file.seek(SeekFrom::Start(1024)).is_err()
            || file.read_exact(&mut superblock).is_err()
            || superblock[56..58] != [0x53, 0xef]
        {
            continue;
        }
        let found = Uuid::from_slice(&superblock[104..120])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if found == expected {
            return Ok(device);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("ext4 filesystem UUID {expected} was not found"),
    ))
}

fn initialize_database(mount_path: &Path) -> io::Result<()> {
    chown(mount_path, Some(AGENT_UID), Some(AGENT_GID))?;
    let database = mount_path.join("state.db");
    let connection = Connection::open(&database).map_err(io::Error::other)?;
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(io::Error::other)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(io::Error::other("SQLite refused WAL journal mode"));
    }
    connection
        .execute_batch("PRAGMA synchronous = FULL;")
        .map_err(io::Error::other)?;
    drop(connection);
    chown(&database, Some(AGENT_UID), Some(AGENT_GID))
}

// Mounting is a privileged operation inside the guest, isolated from the host.
#[allow(unsafe_code)]
pub fn mount_ext4(source: &Path, target: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "disk path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: all pointers refer to live NUL-terminated strings and the data
    // pointer is null.
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

// Unmounting flushes completed SQLite writes before VM teardown.
#[allow(unsafe_code)]
pub fn unmount(target: &Path) -> io::Result<()> {
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: `target` is a live NUL-terminated path and no flags are used.
    let result = unsafe { libc::umount2(target.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn connect_control() -> io::Result<File> {
    let deadline = Instant::now() + CONTROL_CONNECT_TIMEOUT;
    loop {
        match vsock::connect_host(vm_libkrun::protocol::GUEST_VSOCK_PORT) {
            Ok(stream) => return Ok(stream),
            Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

// Mounting is a privileged operation inside the guest, isolated from the host.
#[allow(unsafe_code)]
pub fn mount_virtiofs(tag: &str, guest_path: &Path, read_only: bool) -> io::Result<()> {
    std::fs::create_dir_all(guest_path)?;
    let source = CString::new(tag)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount tag contains NUL"))?;
    let target = CString::new(guest_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    let filesystem = c"virtiofs";
    let mut flags = libc::MS_NOSUID | libc::MS_NODEV;
    if read_only {
        flags |= libc::MS_RDONLY;
    }
    // SAFETY: all pointers refer to live NUL-terminated strings, the optional
    // data pointer is null, and mount flags contain only Linux constants.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            flags,
            std::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
