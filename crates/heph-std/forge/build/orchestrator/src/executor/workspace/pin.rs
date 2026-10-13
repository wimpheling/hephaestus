//! Original directory descriptors; cooperative trusted-root protection only.
use super::super::owner::lock;
use super::super::{BuildExecutionError, cleanup_workspace, filesystem};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(in super::super) struct WorkspacePin {
    original: File,
    path: Mutex<Option<PathBuf>>,
    parent: Option<Arc<Self>>,
    owner: (u32, u32),
    mode: u32,
}
impl WorkspacePin {
    pub(in super::super) fn open(
        path: &Path,
        parent: Option<Arc<Self>>,
    ) -> Result<Arc<Self>, BuildExecutionError> {
        if let Some(parent) = &parent {
            parent.recheck()?;
        }
        let mut options = OpenOptions::new();
        options.read(true);
        // Linux O_NOFOLLOW/O_DIRECTORY are deliberately target-gated, not
        // silently applied as platform-independent numeric flags.
        #[cfg(target_os = "linux")]
        options.custom_flags(0x20_000 | 0x1_0000);
        #[cfg(not(target_os = "linux"))]
        return Err(BuildExecutionError::InvalidConfiguration);
        let original = options.open(path).map_err(filesystem)?;
        let metadata = original.metadata().map_err(filesystem)?;
        let owner = (metadata.uid(), metadata.gid());
        let mode = metadata.mode();
        let pin = Arc::new(Self {
            original,
            owner,
            mode,
            path: Mutex::new(Some(path.to_path_buf())),
            parent,
        });
        pin.recheck()?;
        Ok(pin)
    }
    pub(in super::super) fn recheck(&self) -> Result<(), BuildExecutionError> {
        if let Some(parent) = &self.parent {
            parent.recheck()?;
        }
        let guard = lock(&self.path);
        let result = self.compare(guard.as_ref().ok_or(BuildExecutionError::UnsafeWorkspace)?);
        drop(guard);
        result
    }
    fn compare(&self, path: &Path) -> Result<(), BuildExecutionError> {
        let original = self.original.metadata().map_err(filesystem)?;
        let current = fs::symlink_metadata(path).map_err(filesystem)?;
        if !original.is_dir()
            || !current.is_dir()
            || current.file_type().is_symlink()
            || current.dev() != original.dev()
            || current.ino() != original.ino()
            || (current.uid(), current.gid()) != self.owner
            || (original.uid(), original.gid()) != self.owner
            || current.mode() != self.mode
            || original.mode() != self.mode
            || current.mode() & 0o022 != 0
        {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
        if let Some(parent) = &self.parent {
            if original.uid() != parent.original.metadata().map_err(filesystem)?.uid()
                || current.mode() & 0o777 != 0o700
            {
                return Err(BuildExecutionError::UnsafeWorkspace);
            }
        }
        Ok(())
    }
    pub(in super::super) fn rename(
        &self,
        from: &Path,
        to: &Path,
    ) -> Result<(), BuildExecutionError> {
        if let Some(parent) = &self.parent {
            parent.recheck()?;
        }
        let mut path = lock(&self.path);
        if path.as_deref() != Some(from) {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
        self.compare(from)?;
        match fs::symlink_metadata(to) {
            Ok(_) => return Err(BuildExecutionError::UnsafeWorkspace),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        fs::rename(from, to).map_err(filesystem)?;
        *path = Some(to.to_path_buf());
        let result = self.compare(to);
        drop(path);
        result
    }
    pub(in super::super) fn remove(&self) -> Result<(), BuildExecutionError> {
        let parent = self
            .parent
            .as_ref()
            .ok_or(BuildExecutionError::UnsafeWorkspace)?;
        parent.recheck()?;
        let mut path = lock(&self.path);
        let Some(current) = path.as_ref() else {
            return Ok(());
        };
        self.compare(current)?;
        // Linux lacks atomic inode-conditional recursive unlink against a
        // malicious same-UID writer. Original FD + checks are cooperative only.
        cleanup_workspace(current)?;
        if current.try_exists().map_err(filesystem)? {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
        *path = None;
        drop(path);
        Ok(())
    }
    pub(in super::super) fn removed(&self) -> bool {
        lock(&self.path).is_none()
    }
}
