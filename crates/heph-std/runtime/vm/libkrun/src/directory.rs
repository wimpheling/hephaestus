//! Exact allocated directory custody; observed replacement is never adoption.
use rustix::fs::{AtFlags, Dir, Mode, OFlags};
use std::{
    fs::File,
    io,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub struct ChildDirectory {
    parent: Arc<File>,
    name: String,
    // Installed synchronously after exclusive mkdir. If opening fails, the
    // shared unresolved allocation remains retained and cannot authorize IO.
    child: Arc<Mutex<Option<Arc<File>>>>,
    removed: Arc<AtomicBool>,
}

impl ChildDirectory {
    pub fn create(
        root: &Path,
        name: &str,
        mode: Mode,
        acquired: impl FnOnce(Self),
    ) -> io::Result<Self> {
        let parent: File = rustix::fs::open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        rustix::fs::mkdirat(&parent, name, mode)?;
        let allocation = Self {
            parent: Arc::new(parent),
            name: name.to_owned(),
            child: Arc::new(Mutex::new(None)),
            removed: Arc::new(AtomicBool::new(false)),
        };
        // Publish the exclusive allocation before any subsequent syscall can
        // fail. mkdir/open is not atomic against hostile same-UID substitution.
        acquired(allocation.clone());
        let child: File = rustix::fs::openat(
            &*allocation.parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        *allocation.child.lock().map_err(lock_error)? = Some(Arc::new(child));
        allocation.validate_named()?;
        Ok(allocation)
    }

    pub fn file(&self) -> io::Result<Arc<File>> {
        let file = self
            .child
            .lock()
            .map_err(lock_error)?
            .clone()
            .ok_or_else(|| {
                io::Error::other(
                    "exclusive directory allocation lacks a confirmed child descriptor",
                )
            })?;
        if file.metadata()?.uid() != self.parent.metadata()?.uid() {
            return Err(io::Error::other(
                "allocated child directory UID differs from its parent",
            ));
        }
        Ok(file)
    }

    #[cfg(test)]
    pub fn path(&self) -> io::Result<PathBuf> {
        let directory = self.file()?;
        Ok(fd_path(&directory))
    }

    // False requires the original open directory itself to have been unlinked;
    // a renamed child or foreign replacement cannot establish cleanup.
    pub fn validate_named(&self) -> io::Result<bool> {
        if self.removed.load(Ordering::Acquire) {
            return Ok(false);
        }
        let child = self.file()?;
        let expected = child.metadata()?;
        let named = match rustix::fs::openat(
            &*self.parent,
            self.name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) if expected.nlink() == 0 => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        let observed = named.metadata()?;
        if expected.dev() != observed.dev() || expected.ino() != observed.ino() {
            return Err(io::Error::other(
                "allocated directory named entry was replaced",
            ));
        }
        Ok(true)
    }

    pub fn remove(&self, recursive: bool) -> io::Result<()> {
        if self.removed.load(Ordering::Acquire) {
            return Ok(());
        }
        if !self.validate_named()? {
            self.removed.store(true, Ordering::Release);
            return Ok(());
        }
        if recursive {
            let directory = self.file()?;
            remove_contents(&directory)?;
        }
        // Linux has no identity-conditioned unlinkat. This check detects an
        // observed replacement, not a malicious same-UID check/unlink race.
        if self.validate_named()? {
            rustix::fs::unlinkat(&*self.parent, self.name.as_str(), AtFlags::REMOVEDIR)?;
        }
        self.removed.store(true, Ordering::Release);
        Ok(())
    }
}

pub fn fd_path(file: &File) -> PathBuf {
    // '/.' preserves the existing no-symlink final-component directory checks.
    PathBuf::from(format!("/proc/self/fd/{}/.", file.as_raw_fd()))
}

fn remove_contents(directory: &File) -> io::Result<()> {
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let before = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        let is_directory = before.st_mode & libc::S_IFMT == libc::S_IFDIR;
        if is_directory {
            let child: File = rustix::fs::openat(
                directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?
            .into();
            let metadata = child.metadata()?;
            if metadata.dev() != before.st_dev || metadata.ino() != before.st_ino {
                return Err(io::Error::other(
                    "runtime child changed before descriptor acquisition",
                ));
            }
            remove_contents(&child)?;
        }
        let after = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if before.st_dev != after.st_dev || before.st_ino != after.st_ino {
            return Err(io::Error::other("runtime entry changed before removal"));
        }
        rustix::fs::unlinkat(
            directory,
            name,
            if is_directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            },
        )?;
    }
    Ok(())
}

fn lock_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
#[path = "directory/tests.rs"]
mod tests;
