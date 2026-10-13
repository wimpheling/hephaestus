//! Registered original blocking preparation/mkfs handle and FD leaves.
use super::{
    OciWorkerError, lock,
    mkfs::Original,
    node::{CleanupTask, Node},
    pin::Pin,
    task::Task,
};
use crate::{
    constants::{BUILDER_SCRATCH_BYTES, ScratchDisk},
    filesystem::{prepare_empty_directory, scratch_command},
};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, sync::Arc};
pub(super) async fn prepare(
    operation: &crate::VmOciOperation,
    node: &Arc<Node>,
) -> Result<Arc<ScratchDisk>, OciWorkerError> {
    let owner = node.clone();
    let roots = operation.custody.clone();
    let program = operation.mkfs_ext4.clone();
    let (release, registered) = tokio::sync::oneshot::channel();
    let handle = tokio::task::spawn_blocking(move || {
        if let Err(error) = registered.blocking_recv() {
            owner.error(Arc::new(error));
            return Err(Arc::new(OciWorkerError::InvalidConfiguration));
        }
        inner(&owner, &roots, &program).map_err(Arc::new)
    });
    let task = Task::new(handle);
    lock(&node.state).preparation = Some(task.clone());
    let _registered = release.send(());
    task.join(node).await.map_or_else(
        |_| Err(super::errors::failed("original preparation")),
        |result| result.map_err(super::errors::observed),
    )
}
fn inner(
    node: &Arc<Node>,
    owner: &super::Owner,
    program: &std::path::Path,
) -> Result<Arc<ScratchDisk>, OciWorkerError> {
    owner.roots.recheck()?;
    node.acquire(0)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&node.paths[0])
        .map_err(OciWorkerError::Filesystem)?;
    let file = node.capture_fd(0, file);
    let scratch = Pin::scratch(file, &node.paths[0], owner.roots.scratch.clone())?;
    node.leaf(0, scratch.clone());
    scratch.set_len(BUILDER_SCRATCH_BYTES)?;
    let filesystem_uuid = uuid::Uuid::new_v4();
    owner.roots.program.recheck()?;
    let mut command = scratch_command(program, &filesystem_uuid.to_string(), scratch.path());
    let original = Original::new(&command);
    node.mkfs(original.clone())?;
    if !original.run(&mut command)?.success() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    scratch.recheck()?;
    for (index, root) in [
        (1, owner.roots.candidate.clone()),
        (2, owner.roots.verification.clone()),
    ] {
        root.recheck()?;
        node.acquire(index)?;
        prepare_empty_directory(&node.paths[index])?;
        let file = node.capture_fd(index, Pin::directory_file(&node.paths[index])?);
        let pin = Pin::directory_acquired(file, &node.paths[index], root)?;
        node.leaf(index, pin);
    }
    owner.roots.recheck()?;
    Ok(Arc::new(ScratchDisk {
        path: node.paths[0].clone(),
        filesystem_uuid,
    }))
}
impl Node {
    pub(super) fn start_stop_mkfs(self: &Arc<Self>) -> Option<CleanupTask> {
        let mut tasks = lock(&self.mkfs_cleanup);
        if let Some(current) = tasks.last()
            && !current.joined()
        {
            return Some(current.clone());
        }
        let original = { lock(&self.state).mkfs.clone() }?;
        let owner = self.clone();
        let (release, registered) = tokio::sync::oneshot::channel();
        let task = Task::new(tokio::task::spawn_blocking(move || {
            if let Err(error) = registered.blocking_recv() {
                owner.error(Arc::new(error));
                return Err(Arc::new(OciWorkerError::InvalidConfiguration));
            }
            original.stop_and_join().map_err(Arc::new)
        }));
        tasks.push(task.clone());
        let _registered = release.send(());
        drop(tasks);
        Some(task)
    }
    pub(super) async fn join_formatter(
        self: &Arc<Self>,
        task: Arc<Task<Result<(), Arc<OciWorkerError>>>>,
    ) -> Result<(), OciWorkerError> {
        match task.join(self).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.error(error.clone());
                Err(super::errors::observed(error))
            }
            Err(_) => Err(super::errors::failed("original formatter cleanup")),
        }
    }
}
