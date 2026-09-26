use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::{PreparedRunRuntime, RunRuntimeError, RunRuntimeManager};
use runtime_types::RunId;
use std::fs;
use uuid::Uuid;

use crate::{
    filesystem::{catalog, filesystem, make_tree_removable, runtime_error},
    types::LocalRunRuntimeManager,
};

#[async_trait]
impl RunRuntimeManager for LocalRunRuntimeManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        if self.active_path(run.id).exists() {
            return Err(runtime_error("run runtime already exists"));
        }
        let input = self.load(run).await?;
        self.materialize(run, &input)
    }

    async fn destroy(&self, run_id: RunId) -> Result<(), RunRuntimeError> {
        let path = self.active_path(run_id);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
                make_tree_removable(&path)?;
                fs::remove_dir_all(path).map_err(filesystem)
            }
            Ok(_) => Err(runtime_error("run runtime root is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        let active = self.config.runtime_root.join("active");
        let mut recovered = 0;
        for entry in fs::read_dir(active).map_err(filesystem)? {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("run runtime entry is invalid"))?;
            let uuid = Uuid::parse_str(&name)
                .map_err(|_| runtime_error("run runtime entry is invalid"))?;
            let live = self
                .catalog
                .run_is_live(RunId::from_uuid(uuid))
                .await
                .map_err(catalog)?;
            if !live {
                self.destroy(RunId::from_uuid(uuid)).await?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
}
