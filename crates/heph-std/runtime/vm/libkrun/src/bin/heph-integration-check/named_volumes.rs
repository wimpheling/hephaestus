use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        rustix::process::geteuid().as_raw(),
        0,
        "privileged fixture probe"
    );
    if std::env::var("HEPH_TEST_DIRTY_MARKER").as_deref() == Ok("1") {
        fs::write(
            "/volumes/rw/dirty-workload-executed",
            b"unexpected-workload",
        )?;
    }
    let read_only = Path::new("/volumes/ro");
    let writable = Path::new("/volumes/rw");
    let facts_device = device(read_only, "HEPH_TEST_RO_UUID", true)?;
    let data_device = device(writable, "HEPH_TEST_RW_UUID", false)?;
    assert_eq!(fs::read(read_only.join("sentinel"))?, b"immutable-facts\n");
    assert_eq!(fs::read(writable.join("sentinel"))?, b"mutable-data\n");
    let file_error = fs::write(read_only.join("forbidden"), b"must-fail").unwrap_err();
    assert_eq!(file_error.raw_os_error(), Some(libc::EROFS));
    let raw_error = OpenOptions::new()
        .write(true)
        .open(&facts_device)
        .and_then(|mut block| block.write_all(&[0]))
        .unwrap_err();
    // Block-driver denial codes differ. The identical writable operation below
    // is the privilege control; report the actual denial for diagnosis.
    println!("named-ro-raw-denial={raw_error}");
    // The same privileged process can remount the writable control, proving
    // that failure on read-only data is not missing mount privilege.
    remount_writable(writable)?;
    assert!(remount_writable(read_only).is_err());
    let mut control = File::open(&data_device)?;
    let mut original = [0_u8; 1];
    control.read_exact(&mut original)?;
    block_set_read_only(&control, false)?;
    drop(control);
    let mut block = OpenOptions::new().write(true).open(&data_device)?;
    block.write_all(&original)?;
    block.sync_all()?;
    drop(block);
    let (accepted, effective) = reject_backend_write_after_ro_clear(&facts_device)?;
    fs::write(writable.join("persisted"), b"named-writable-persisted\n")?;
    assert!(!Path::new("/var/lib/hephaestus/state.db").exists());
    println!(
        "NAMED_VOLUME_GUEST=1 uid=0 paths_uuid=1 sysfs_ro=1 blkroget=1 ro_file_denied=1 ro_raw_denied=1 ro_remount_denied=1 rw_remount_control=1 rw_raw_control=1 rw_ioctl_control=1 ro_backend_durable_denied=1 ro_clear_accepted={accepted} ro_clear_effective={effective} rw_write=1 no_sqlite_init=1"
    );
    Ok(())
}

fn device(path: &Path, uuid_env: &str, expected_ro: bool) -> io::Result<PathBuf> {
    let device_number = fs::metadata(path)?.dev();
    let expected_uuid = Uuid::parse_str(&std::env::var(uuid_env).map_err(io::Error::other)?)
        .map_err(io::Error::other)?;
    let mut matches = Vec::new();
    for entry in fs::read_dir("/sys/class/block")? {
        let entry = entry?;
        let device = Path::new("/dev").join(entry.file_name());
        if fs::metadata(&device).is_ok_and(|metadata| metadata.rdev() == device_number) {
            assert_eq!(
                fs::read_to_string(entry.path().join("ro"))?.trim(),
                if expected_ro { "1" } else { "0" }
            );
            let mut file = File::open(&device)?;
            assert_eq!(block_read_only(&file)?, expected_ro);
            file.seek(SeekFrom::Start(1024))?;
            let mut superblock = [0_u8; 120];
            file.read_exact(&mut superblock)?;
            assert_eq!(superblock[56..58], [0x53, 0xef]);
            assert_eq!(&superblock[104..120], expected_uuid.as_bytes());
            matches.push(device);
        }
    }
    if matches.len() != 1 {
        return Err(io::Error::other("mounted device identity is not unique"));
    }
    Ok(matches.remove(0))
}

// The native-only probe verifies the Linux block-layer flag directly.
#[allow(unsafe_code)]
fn block_read_only(file: &File) -> io::Result<bool> {
    let mut value: libc::c_int = -1;
    // Linux fs.h defines BLKROGET as _IO(0x12, 94), or 0x125e.
    // SAFETY: the descriptor is open; BLKROGET writes exactly one live c_int.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), 0x125e, &raw mut value) };
    if result == 0 {
        Ok(value == 1)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn reject_backend_write_after_ro_clear(device: &Path) -> io::Result<(bool, bool)> {
    let control = File::open(device)?;
    let accepted = block_set_read_only(&control, false).is_ok();
    let effective = !block_read_only(&control)?;
    let device_name = device
        .file_name()
        .ok_or_else(|| io::Error::other("device name"))?;
    let sysfs = fs::read_to_string(Path::new("/sys/class/block").join(device_name).join("ro"))?;
    println!("named-ro-after-clear-sysfs={}", sysfs.trim());
    let mut original = [0_u8; 1];
    (&control).read_exact(&mut original)?;
    // Byte zero is unused ext4 boot padding in this disposable raw fixture.
    let result = OpenOptions::new()
        .write(true)
        .open(device)
        .and_then(|mut block| {
            block.write_all(&[original[0] ^ 0xff])?;
            block.sync_all()
        });
    block_set_read_only(&control, true)?;
    assert!(block_read_only(&control)?);
    assert!(
        result.is_err(),
        "read-only backend accepted a durable raw write"
    );
    println!("named-ro-after-clear-raw-denial={}", result.unwrap_err());
    Ok((accepted, effective))
}

// Native-only adversarial probe for the Linux BLKROSET ioctl.
#[allow(unsafe_code)]
fn block_set_read_only(file: &File, read_only: bool) -> io::Result<()> {
    let value = libc::c_int::from(read_only);
    // Linux fs.h: BLKROSET = _IO(0x12, 93). SAFETY: the descriptor and one
    // c_int pointer are live for the duration of this synchronous ioctl.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), 0x125d, &raw const value) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// Both calls run only inside the disposable guest with an owned test mount.
#[allow(unsafe_code)]
fn remount_writable(path: &Path) -> io::Result<()> {
    let target = CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
    // SAFETY: target is NUL-terminated and all optional pointers are null.
    let result = unsafe {
        libc::mount(
            std::ptr::null(),
            target.as_ptr(),
            std::ptr::null(),
            libc::MS_REMOUNT,
            std::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
