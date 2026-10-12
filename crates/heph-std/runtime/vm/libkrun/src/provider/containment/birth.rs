//! Exact original allocation record, published durably before any worker spawn.
use super::super::helpers::provider_error;
use rustix::fs::{AtFlags, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use vm_trait::{VmError, VmId, VmProviderOwnerScope};

const LIMIT: u64 = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) uid: u32,
}

impl Identity {
    pub(super) fn read(file: &File) -> Result<Self, VmError> {
        let value = file.metadata().map_err(io_error)?;
        Ok(Self {
            device: value.dev(),
            inode: value.ino(),
            uid: value.uid(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    version: u32,
    birth: uuid::Uuid,
    namespace: String,
    host_id: String,
    vm_id: String,
    runtime_root: Identity,
    cgroup_root: Identity,
    pub(super) runtime: Identity,
    pub(super) cgroup: Identity,
}

pub(in crate::provider) struct BirthRecord {
    root: File,
    file: File,
    name: String,
    pub(super) value: Record,
    bytes: Vec<u8>,
    // A known exclusive creating descriptor may clean its own interrupted write;
    // recovery never accepts a partial record as provenance.
    published: AtomicBool,
}

impl BirthRecord {
    pub(in crate::provider) fn publish(
        roots: (&File, &File),
        scope: &VmProviderOwnerScope,
        id: &VmId,
        children: (&File, &File),
        acquired: impl FnOnce(Arc<Self>),
    ) -> Result<(), VmError> {
        crate::validation::validate_id(id)?;
        let value = Record {
            version: 1,
            birth: uuid::Uuid::new_v4(),
            namespace: scope.namespace().into(),
            host_id: scope.host_id().into(),
            vm_id: id.0.clone(),
            runtime_root: Identity::read(roots.0)?,
            cgroup_root: Identity::read(roots.1)?,
            runtime: Identity::read(children.0)?,
            cgroup: Identity::read(children.1)?,
        };
        let bytes = serde_json::to_vec(&value).map_err(io_error)?;
        if bytes.len() > usize::try_from(LIMIT).expect("bounded record limit") {
            return Err(VmError::InvalidState(
                "VM birth record exceeds bounded format",
            ));
        }
        let name = filename(id);
        let root = roots.0.try_clone().map_err(io_error)?;
        let file: File = rustix::fs::openat(
            &root,
            name.as_str(),
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )
        .map_err(io_error)?
        .into();
        let record = Arc::new(Self {
            root,
            file,
            name,
            value,
            bytes,
            published: AtomicBool::new(false),
        });
        acquired(Arc::clone(&record));
        check_file(&record.file, record.value.runtime_root.uid)?;
        (&record.file).write_all(&record.bytes).map_err(io_error)?;
        record.file.sync_all().map_err(io_error)?;
        record.root.sync_all().map_err(io_error)?;
        record.published.store(true, Ordering::Release);
        Ok(())
    }

    pub(super) fn load(
        roots: (&File, &File),
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<Option<Self>, VmError> {
        let name = filename(id);
        let Some(file) = open_record(roots.0, &name)? else {
            return Ok(None);
        };
        check_file(&file, roots.0.metadata().map_err(io_error)?.uid())?;
        let bytes = read(&file)?;
        let value: Record = serde_json::from_slice(&bytes).map_err(io_error)?;
        if value.version != 1
            || value.birth.is_nil()
            || value.vm_id != id.0
            || value.namespace != scope.namespace()
            || value.host_id != scope.host_id()
            || value.runtime_root != Identity::read(roots.0)?
            || value.cgroup_root != Identity::read(roots.1)?
            || value.runtime.uid != value.runtime_root.uid
            || value.cgroup.uid != value.cgroup_root.uid
        {
            return Err(VmError::InvalidState(
                "original VM birth record differs from owner/id/roots",
            ));
        }
        Ok(Some(Self {
            root: roots.0.try_clone().map_err(io_error)?,
            file,
            name,
            value,
            bytes,
            published: AtomicBool::new(true),
        }))
    }

    pub(in crate::provider) fn remove_original(&self) -> Result<(), VmError> {
        let Some(current) = open_record(&self.root, &self.name)? else {
            if self.file.metadata().map_err(io_error)?.nlink() != 0 {
                return Err(VmError::InvalidState(
                    "original VM birth record was renamed",
                ));
            }
            return self.root.sync_all().map_err(io_error);
        };
        check_file(&current, self.value.runtime_root.uid)?;
        if Identity::read(&current)? != Identity::read(&self.file)? {
            return Err(VmError::InvalidState(
                "original VM birth record was replaced",
            ));
        }
        if self.published.load(Ordering::Acquire) && read(&current)? != self.bytes {
            return Err(VmError::InvalidState(
                "original VM birth record bytes changed",
            ));
        }
        rustix::fs::unlinkat(&self.root, self.name.as_str(), AtFlags::empty()).map_err(io_error)?;
        self.root.sync_all().map_err(io_error)
    }
}

pub(super) fn filename(id: &VmId) -> String {
    format!(".heph-vm-birth-{}.json", id.0)
}

fn open_record(root: &File, name: &str) -> Result<Option<File>, VmError> {
    match rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => Ok(Some(file.into())),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn read(file: &File) -> Result<Vec<u8>, VmError> {
    // Independent open descriptions start at zero; reads do not alter retained
    // creation custody. Callers only read newly opened record descriptions.
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > usize::try_from(LIMIT).expect("bounded record limit") {
        return Err(VmError::InvalidState(
            "VM birth record exceeds bounded format",
        ));
    }
    Ok(bytes)
}

fn check_file(file: &File, uid: u32) -> Result<(), VmError> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(VmError::InvalidState(
            "VM birth record must be a private original single-link file",
        ));
    }
    Ok(())
}

fn io_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    provider_error("vm-birth-record", error)
}
