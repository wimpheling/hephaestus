use std::{
    ffi::{CStr, CString, OsStr},
    path::Path,
    sync::Arc,
};

use crate::ffi::{
    api::{AUTO_FS, DynamicApi, HEPH_INIT, KrunApi, ROOT_BLOCK_ID, ROOT_DEVICE},
    errors::{FfiError, deterministic_mac, path_cstring, status},
};
use crate::validation::{PreparedMount, PreparedNetwork, PreparedRoot, PreparedSpec};

/// Safe owner for one libkrun configuration context.
pub struct Context {
    api: Arc<dyn KrunApi>,
    id: Option<u32>,
}

impl Context {
    pub fn load(library: &OsStr) -> Result<Self, FfiError> {
        let api = Arc::new(DynamicApi::load(library)?);
        Self::from_api(api)
    }

    pub fn from_api(api: Arc<dyn KrunApi>) -> Result<Self, FfiError> {
        let result = api.create_context();
        if result < 0 {
            return Err(FfiError::code("krun_create_ctx", result));
        }
        let id = u32::try_from(result)
            .map_err(|_| FfiError::message("krun_create_ctx", "invalid context identifier"))?;
        Ok(Self { api, id: Some(id) })
    }

    // Keep the FFI setup in call order so failures identify the exact libkrun
    // operation; the sequential resource mapping necessarily exceeds the
    // pedantic line-count threshold.
    #[allow(clippy::too_many_lines)]
    pub fn configure(
        &self,
        spec: &PreparedSpec,
        passt_socket: Option<&Path>,
        control_socket: &Path,
        broker_socket: Option<&Path>,
        private_service_socket: Option<&Path>,
        runtime_git_socket: Option<&Path>,
    ) -> Result<(), FfiError> {
        let id = self.id();
        status(
            "krun_set_vm_config",
            self.api.set_vm_config(id, spec.vcpus, spec.memory_mib),
        )?;

        match &spec.root {
            PreparedRoot::Directory { path } => {
                let path = path_cstring(path)?;
                status("krun_set_root", self.api.set_root(id, &path))?;
            }
            PreparedRoot::RawDisk { path, read_only } => {
                self.add_disk(ROOT_BLOCK_ID, path, *read_only)?;
                status(
                    "krun_set_root_disk_remount",
                    self.api.set_root_disk_remount(id, ROOT_DEVICE, AUTO_FS),
                )?;
            }
        }

        for disk in &spec.disks {
            let id = CString::new(disk.id.as_bytes())
                .map_err(|_| FfiError::message("krun_add_disk3", "disk ID contains NUL"))?;
            self.add_disk(&id, &disk.path, disk.read_only)?;
        }
        for mount in &spec.mounts {
            self.add_mount(mount)?;
        }

        status(
            "krun_disable_implicit_vsock",
            self.api.disable_implicit_vsock(id),
        )?;
        status("krun_add_vsock", self.api.add_vsock(id, 0))?;
        let control_socket = path_cstring(control_socket)?;
        status(
            "krun_add_vsock_port",
            self.api
                .add_vsock_port(id, crate::protocol::GUEST_VSOCK_PORT, &control_socket),
        )?;
        if matches!(spec.network, PreparedNetwork::BrokerOnly) {
            let broker_socket = broker_socket.ok_or_else(|| {
                FfiError::message(
                    "krun_add_vsock_port",
                    "broker-only mode has no host broker socket",
                )
            })?;
            let broker_socket = path_cstring(broker_socket)?;
            status(
                "krun_add_vsock_port",
                self.api.add_vsock_port(
                    id,
                    crate::protocol::SECRET_BROKER_VSOCK_PORT,
                    &broker_socket,
                ),
            )?;
        }
        match (spec.private_http_service.as_ref(), private_service_socket) {
            (Some(_), Some(socket)) => {
                let socket = path_cstring(socket)?;
                status(
                    "krun_add_vsock_port",
                    self.api.add_vsock_port(
                        id,
                        crate::protocol::PRIVATE_SERVICE_VSOCK_PORT,
                        &socket,
                    ),
                )?;
            }
            (Some(_), None) => {
                return Err(FfiError::message(
                    "krun_add_vsock_port",
                    "private service has no host socket",
                ));
            }
            (None, Some(_)) => {
                return Err(FfiError::message(
                    "krun_add_vsock_port",
                    "private service socket provided without a service",
                ));
            }
            (None, None) => {}
        }
        match (spec.runtime_git_bridge.as_ref(), runtime_git_socket) {
            (Some(_), Some(socket)) => {
                let socket = path_cstring(socket)?;
                status(
                    "krun_add_vsock_port",
                    self.api
                        .add_vsock_port(id, crate::protocol::RUNTIME_GIT_VSOCK_PORT, &socket),
                )?;
            }
            (Some(_), None) => {
                return Err(FfiError::message(
                    "krun_add_vsock_port",
                    "runtime Git bridge has no host socket",
                ));
            }
            (None, Some(_)) => {
                return Err(FfiError::message(
                    "krun_add_vsock_port",
                    "runtime Git bridge socket provided without a bridge",
                ));
            }
            (None, None) => {}
        }

        match (&spec.network, passt_socket) {
            (PreparedNetwork::Disabled | PreparedNetwork::BrokerOnly, None) => {}
            (PreparedNetwork::UserMode { .. }, Some(socket)) => {
                let socket = path_cstring(socket)?;
                let mut mac = deterministic_mac(&spec.id);
                status(
                    "krun_add_net_unixstream",
                    self.api.add_net_unixstream(id, &socket, &mut mac),
                )?;
            }
            _ => {
                return Err(FfiError::message(
                    "network configuration",
                    "passt socket does not match requested network mode",
                ));
            }
        }

        status("krun_set_exec", self.api.set_exec(id, HEPH_INIT))
    }

    pub fn start_enter(mut self) -> Result<(), FfiError> {
        let id = self.id.take().expect("context is consumed only once");
        let result = self.api.start_enter(id);
        status("krun_start_enter", result)
    }

    fn add_disk(&self, block_id: &CStr, path: &Path, read_only: bool) -> Result<(), FfiError> {
        let path = path_cstring(path)?;
        status(
            "krun_add_disk3",
            self.api.add_disk(self.id(), block_id, &path, read_only),
        )
    }

    fn add_mount(&self, mount: &PreparedMount) -> Result<(), FfiError> {
        let tag = CString::new(mount.tag.as_bytes())
            .map_err(|_| FfiError::message("krun_add_virtiofs3", "mount tag contains NUL"))?;
        let path = path_cstring(&mount.host_path)?;
        status(
            "krun_add_virtiofs3",
            self.api
                .add_virtio_fs(self.id(), &tag, &path, mount.read_only),
        )
    }

    const fn id(&self) -> u32 {
        self.id.expect("live context")
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _result = self.api.free_context(id);
        }
    }
}
