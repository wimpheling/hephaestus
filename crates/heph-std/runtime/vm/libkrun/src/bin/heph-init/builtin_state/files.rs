use rustix::fs::{FileType, Gid, Mode, OFlags, Uid, fchown, fstat, openat};
use std::{io, os::fd::OwnedFd};

/// Opens only a single regular inode inside the pinned filesystem.
pub fn regular_file(directory: &OwnedFd, name: &str) -> io::Result<Option<OwnedFd>> {
    // O_PATH observes the inode without opening a FIFO/device for IO. NOFOLLOW
    // leaves symlinks identifiable instead of traversing them.
    let metadata = match openat(
        directory,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let stat = fstat(&metadata)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "built-in database or sidecar is not a single regular inode",
        ));
    }
    let file = openat(
        directory,
        name,
        OFlags::RDWR | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let opened = fstat(&file)?;
    if opened.st_dev != stat.st_dev
        || opened.st_ino != stat.st_ino
        || FileType::from_raw_mode(opened.st_mode) != FileType::RegularFile
        || opened.st_nlink != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "built-in database inode changed before open",
        ));
    }
    drop(metadata);
    Ok(Some(file))
}

pub fn give_to_agent(file: &OwnedFd, uid: u32, gid: u32) -> io::Result<()> {
    fchown(file, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))?;
    Ok(())
}
