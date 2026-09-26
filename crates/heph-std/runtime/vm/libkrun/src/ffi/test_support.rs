use super::*;
use crate::validation::{
    PreparedCommand, PreparedDisk, PreparedForward, PreparedMount, PreparedNetwork, PreparedRoot,
    PreparedSpec,
};
use std::{
    collections::BTreeMap,
    ffi::CStr,
    net::{IpAddr, Ipv4Addr},
    path::Path,
    sync::{Mutex, MutexGuard},
};

pub(super) fn prepared_spec() -> PreparedSpec {
    PreparedSpec {
        id: String::from("recording"),
        root: PreparedRoot::Directory {
            path: Path::new("/images/root").to_path_buf(),
        },
        disks: vec![PreparedDisk {
            id: String::from("sqlite"),
            path: Path::new("/disks/sqlite.raw").to_path_buf(),
            read_only: false,
        }],
        mounts: vec![PreparedMount {
            tag: String::from("workspace"),
            host_path: Path::new("/mounts/workspace").to_path_buf(),
            guest_path: Path::new("/workspace").to_path_buf(),
            read_only: false,
        }],
        vcpus: 2,
        memory_mib: 1024,
        network: PreparedNetwork::UserMode {
            ingress: vec![PreparedForward {
                bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                host_port: 0,
                guest_port: 8080,
            }],
        },
        command: PreparedCommand {
            program: String::from("/bin/true"),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: Some(Path::new("/").to_path_buf()),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Call {
    Create,
    Free {
        id: u32,
    },
    VmConfig {
        id: u32,
        vcpus: u8,
        memory_mib: u32,
    },
    Root {
        id: u32,
        path: String,
    },
    Disk {
        id: u32,
        block_id: String,
        path: String,
        read_only: bool,
    },
    RootRemount {
        id: u32,
        device: String,
        filesystem: String,
    },
    VirtioFs {
        id: u32,
        tag: String,
        path: String,
        read_only: bool,
    },
    Network {
        id: u32,
        path: String,
        mac: [u8; 6],
    },
    DisableImplicitVsock {
        id: u32,
    },
    Vsock {
        id: u32,
        cid: u32,
    },
    VsockPort {
        id: u32,
        port: u32,
        path: String,
    },
    Exec {
        id: u32,
        executable: String,
    },
    Start {
        id: u32,
    },
}

pub(super) struct RecordingApi {
    calls: Mutex<Vec<Call>>,
    fail_operation: Option<&'static str>,
}

impl RecordingApi {
    pub(super) const fn new(fail_operation: Option<&'static str>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_operation,
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }

    fn record(&self, operation: &'static str, call: Call) -> i32 {
        lock(&self.calls).push(call);
        if self.fail_operation == Some(operation) {
            -22
        } else {
            0
        }
    }
}

impl KrunApi for RecordingApi {
    fn create_context(&self) -> i32 {
        if self.record("create", Call::Create) < 0 {
            -22
        } else {
            7
        }
    }

    fn free_context(&self, id: u32) -> i32 {
        self.record("free", Call::Free { id })
    }

    fn set_vm_config(&self, id: u32, vcpus: u8, memory_mib: u32) -> i32 {
        self.record(
            "vm-config",
            Call::VmConfig {
                id,
                vcpus,
                memory_mib,
            },
        )
    }

    fn set_root(&self, id: u32, path: &CStr) -> i32 {
        self.record(
            "root",
            Call::Root {
                id,
                path: text(path),
            },
        )
    }

    fn add_disk(&self, id: u32, block_id: &CStr, path: &CStr, read_only: bool) -> i32 {
        self.record(
            "disk",
            Call::Disk {
                id,
                block_id: text(block_id),
                path: text(path),
                read_only,
            },
        )
    }

    fn set_root_disk_remount(&self, id: u32, device: &CStr, filesystem: &CStr) -> i32 {
        self.record(
            "root-remount",
            Call::RootRemount {
                id,
                device: text(device),
                filesystem: text(filesystem),
            },
        )
    }

    fn add_virtio_fs(&self, id: u32, tag: &CStr, path: &CStr, read_only: bool) -> i32 {
        self.record(
            "virtio-fs",
            Call::VirtioFs {
                id,
                tag: text(tag),
                path: text(path),
                read_only,
            },
        )
    }

    fn add_net_unixstream(&self, id: u32, path: &CStr, mac: &mut [u8; 6]) -> i32 {
        self.record(
            "network",
            Call::Network {
                id,
                path: text(path),
                mac: *mac,
            },
        )
    }

    fn disable_implicit_vsock(&self, id: u32) -> i32 {
        self.record("disable-vsock", Call::DisableImplicitVsock { id })
    }

    fn add_vsock(&self, id: u32, cid: u32) -> i32 {
        self.record("vsock", Call::Vsock { id, cid })
    }

    fn add_vsock_port(&self, id: u32, port: u32, path: &CStr) -> i32 {
        self.record(
            "vsock-port",
            Call::VsockPort {
                id,
                port,
                path: text(path),
            },
        )
    }

    fn set_exec(&self, id: u32, executable: &CStr) -> i32 {
        self.record(
            "exec",
            Call::Exec {
                id,
                executable: text(executable),
            },
        )
    }

    fn start_enter(&self, id: u32) -> i32 {
        self.record("start", Call::Start { id })
    }
}

fn text(value: &CStr) -> String {
    value.to_string_lossy().into_owned()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
