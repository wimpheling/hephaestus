//! Original FD binding; cooperative trusted-root boundary, not hostile UID exclusion.
use super::{OciWorkerError, lock};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Copy)]
enum Kind {
    Directory,
    File,
}
pub(super) struct Pin {
    original: Arc<File>,
    path: PathBuf,
    parent: Option<Arc<Self>>,
    kind: Kind,
    identity: (u64, u64, u32, u32),
    mode: Mutex<u32>,
    removed: AtomicBool,
}
pub(super) struct Roots {
    pub(super) candidate: Arc<Pin>,
    pub(super) scratch: Arc<Pin>,
    pub(super) verification: Arc<Pin>,
    pub(super) program: Arc<Pin>,
}
impl Roots {
    pub(super) fn open(
        candidate: &Path,
        scratch: &Path,
        verification: &Path,
        program: &Path,
    ) -> Result<Self, OciWorkerError> {
        Ok(Self {
            candidate: Pin::directory(candidate, None)?,
            scratch: Pin::directory(scratch, None)?,
            verification: Pin::directory(verification, None)?,
            program: Pin::program(program)?,
        })
    }
    pub(super) fn recheck(&self) -> Result<(), OciWorkerError> {
        for pin in [
            &self.candidate,
            &self.scratch,
            &self.verification,
            &self.program,
        ] {
            pin.recheck()?;
        }
        Ok(())
    }
}
impl Pin {
    fn acquire(
        file: Arc<File>,
        path: &Path,
        parent: Option<Arc<Self>>,
        kind: Kind,
    ) -> Result<Arc<Self>, OciWorkerError> {
        let metadata = file.metadata().map_err(OciWorkerError::Filesystem)?;
        let pin = Arc::new(Self {
            original: file,
            path: path.to_path_buf(),
            parent,
            kind,
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.uid(),
                metadata.gid(),
            ),
            mode: Mutex::new(metadata.mode()),
            removed: AtomicBool::new(false),
        });
        pin.recheck()?;
        Ok(pin)
    }
    pub(super) fn directory(
        path: &Path,
        parent: Option<Arc<Self>>,
    ) -> Result<Arc<Self>, OciWorkerError> {
        if let Some(parent) = &parent {
            parent.recheck()?;
        }
        let mut options = OpenOptions::new();
        options.read(true);
        // Linux O_NOFOLLOW/O_DIRECTORY: deliberately target-gated.
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20_000 | 0x1_0000);
        #[cfg(not(target_os = "linux"))]
        return Err(OciWorkerError::InvalidConfiguration);
        Self::acquire(
            Arc::new(options.open(path).map_err(OciWorkerError::Filesystem)?),
            path,
            parent,
            Kind::Directory,
        )
    }
    pub(super) fn directory_file(path: &Path) -> Result<File, OciWorkerError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20_000 | 0x1_0000);
        #[cfg(not(target_os = "linux"))]
        return Err(OciWorkerError::InvalidConfiguration);
        options.open(path).map_err(OciWorkerError::Filesystem)
    }
    pub(super) fn directory_acquired(
        file: Arc<File>,
        path: &Path,
        parent: Arc<Self>,
    ) -> Result<Arc<Self>, OciWorkerError> {
        Self::acquire(file, path, Some(parent), Kind::Directory)
    }
    fn program(path: &Path) -> Result<Arc<Self>, OciWorkerError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20_000);
        #[cfg(not(target_os = "linux"))]
        return Err(OciWorkerError::InvalidConfiguration);
        Self::acquire(
            Arc::new(options.open(path).map_err(OciWorkerError::Filesystem)?),
            path,
            None,
            Kind::File,
        )
    }
    pub(super) fn scratch(
        file: Arc<File>,
        path: &Path,
        parent: Arc<Self>,
    ) -> Result<Arc<Self>, OciWorkerError> {
        Self::acquire(file, path, Some(parent), Kind::File)
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn recheck(&self) -> Result<(), OciWorkerError> {
        if self.removed.load(Ordering::SeqCst) {
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        if let Some(parent) = &self.parent {
            parent.recheck()?;
        }
        self.compare()
    }
    fn compare(&self) -> Result<(), OciWorkerError> {
        let original = self
            .original
            .metadata()
            .map_err(OciWorkerError::Filesystem)?;
        let named = fs::symlink_metadata(&self.path).map_err(OciWorkerError::Filesystem)?;
        let mode = *lock(&self.mode);
        for metadata in [&original, &named] {
            let kind = match self.kind {
                Kind::Directory => metadata.is_dir(),
                Kind::File => metadata.is_file(),
            };
            if !kind
                || metadata.file_type().is_symlink()
                || (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.uid(),
                    metadata.gid(),
                ) != self.identity
                || metadata.mode() != mode
                || mode & 0o022 != 0
            {
                return Err(OciWorkerError::UnsafeMaterializationPath);
            }
            if let Some(parent) = &self.parent {
                if metadata.uid() != parent.identity.2 || metadata.dev() != parent.identity.0 {
                    return Err(OciWorkerError::UnsafeMaterializationPath);
                }
            }
        }
        Ok(())
    }
    pub(super) fn set_len(&self, length: u64) -> Result<(), OciWorkerError> {
        self.recheck()?;
        self.original
            .set_len(length)
            .map_err(OciWorkerError::Filesystem)
    }
    pub(super) fn seal_candidate(&self) -> Result<(), OciWorkerError> {
        self.recheck()?;
        crate::filesystem::seal_candidate_layout(&self.path)?;
        // Sole declared source transition of this SAME original directory.
        let mut mode = lock(&self.mode);
        *mode = (*mode & !0o777) | 0o500;
        drop(mode);
        self.recheck()
    }
    pub(super) fn remove_scratch(&self) -> Result<(), OciWorkerError> {
        if self.removed.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.recheck()?;
        if !matches!(self.kind, Kind::File) {
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        let parent = self
            .parent
            .as_ref()
            .ok_or(OciWorkerError::UnsafeMaterializationPath)?;
        let name = self
            .path
            .file_name()
            .ok_or(OciWorkerError::UnsafeMaterializationPath)?;
        // Descriptor-relative original directory. Linux still offers no atomic
        // inode-conditional unlink against a malicious same-UID replacement.
        let relative =
            PathBuf::from(format!("/proc/self/fd/{}", parent.original.as_raw_fd())).join(name);
        fs::remove_file(relative).map_err(OciWorkerError::Filesystem)?;
        self.removed.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub(super) fn removed(&self) -> bool {
        self.removed.load(Ordering::SeqCst)
    }
}
