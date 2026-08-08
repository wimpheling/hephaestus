// libkrun's stable C ABI necessarily requires raw function pointers. This is
// the crate's only unsafe module; every call documents its pointer invariants.
#![allow(unsafe_code)]

use crate::validation::{PreparedMount, PreparedNetwork, PreparedRoot, PreparedSpec};
use libloading::Library;
use std::{
    ffi::{CStr, CString, OsStr},
    fmt,
    os::unix::ffi::OsStrExt,
    path::Path,
    sync::Arc,
};

const DISK_FORMAT_RAW: u32 = 0;
const SYNC_FULL: u32 = 2;
const NET_FLAG_DHCP_CLIENT: u32 = 1 << 1;
const COMPAT_NET_FEATURES: u32 = (1 << 0) | (1 << 1) | (1 << 7) | (1 << 10) | (1 << 11) | (1 << 14);
const ROOT_BLOCK_ID: &CStr = c"root";
const ROOT_DEVICE: &CStr = c"/dev/vda";
const AUTO_FS: &CStr = c"auto";
const HEPH_INIT: &CStr = c"/usr/libexec/hephaestus/heph-init";

type CreateContext = unsafe extern "C" fn() -> i32;
type FreeContext = unsafe extern "C" fn(u32) -> i32;
type SetVmConfig = unsafe extern "C" fn(u32, u8, u32) -> i32;
type SetRoot = unsafe extern "C" fn(u32, *const std::ffi::c_char) -> i32;
type AddDisk = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
    u32,
    bool,
    bool,
    u32,
) -> i32;
type SetRootDiskRemount = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
) -> i32;
type AddVirtioFs =
    unsafe extern "C" fn(u32, *const std::ffi::c_char, *const std::ffi::c_char, u64, bool) -> i32;
type AddNetUnixStream =
    unsafe extern "C" fn(u32, *const std::ffi::c_char, i32, *mut u8, u32, u32) -> i32;
type DisableImplicitVsock = unsafe extern "C" fn(u32) -> i32;
type AddVsock = unsafe extern "C" fn(u32, u32) -> i32;
type AddVsockPort = unsafe extern "C" fn(u32, u32, *const std::ffi::c_char) -> i32;
type SetExec = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const *const std::ffi::c_char,
    *const *const std::ffi::c_char,
) -> i32;
type StartEnter = unsafe extern "C" fn(u32) -> i32;

struct DynamicApi {
    _library: Library,
    create_context: CreateContext,
    free_context: FreeContext,
    set_vm_config: SetVmConfig,
    set_root: SetRoot,
    add_disk: AddDisk,
    set_root_disk_remount: SetRootDiskRemount,
    add_virtio_fs: AddVirtioFs,
    add_net_unixstream: AddNetUnixStream,
    disable_implicit_vsock: DisableImplicitVsock,
    add_vsock: AddVsock,
    add_vsock_port: AddVsockPort,
    set_exec: SetExec,
    start_enter: StartEnter,
}

trait KrunApi: Send + Sync {
    fn create_context(&self) -> i32;
    fn free_context(&self, id: u32) -> i32;
    fn set_vm_config(&self, id: u32, vcpus: u8, memory_mib: u32) -> i32;
    fn set_root(&self, id: u32, path: &CStr) -> i32;
    fn add_disk(&self, id: u32, block_id: &CStr, path: &CStr, read_only: bool) -> i32;
    fn set_root_disk_remount(&self, id: u32, device: &CStr, filesystem: &CStr) -> i32;
    fn add_virtio_fs(&self, id: u32, tag: &CStr, path: &CStr, read_only: bool) -> i32;
    fn add_net_unixstream(&self, id: u32, path: &CStr, mac: &mut [u8; 6]) -> i32;
    fn disable_implicit_vsock(&self, id: u32) -> i32;
    fn add_vsock(&self, id: u32, cid: u32) -> i32;
    fn add_vsock_port(&self, id: u32, port: u32, path: &CStr) -> i32;
    fn set_exec(&self, id: u32, executable: &CStr) -> i32;
    fn start_enter(&self, id: u32) -> i32;
}

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

    fn from_api(api: Arc<dyn KrunApi>) -> Result<Self, FfiError> {
        let result = api.create_context();
        if result < 0 {
            return Err(FfiError::code("krun_create_ctx", result));
        }
        let id = u32::try_from(result)
            .map_err(|_| FfiError::message("krun_create_ctx", "invalid context identifier"))?;
        Ok(Self { api, id: Some(id) })
    }

    pub fn configure(
        &self,
        spec: &PreparedSpec,
        passt_socket: Option<&Path>,
        control_socket: &Path,
        broker_socket: Option<&Path>,
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

impl DynamicApi {
    fn load(path: &OsStr) -> Result<Self, FfiError> {
        // SAFETY: loading a configured shared object can run library
        // constructors. The worker is the dedicated trust boundary for
        // libkrun/libkrunfw, and the path is administrator configuration.
        let library = unsafe { Library::new(path) }
            .map_err(|error| FfiError::message("load libkrun", error.to_string()))?;
        // SAFETY: each name and function-pointer type below is copied directly
        // from stable libkrun 1.x's public `libkrun.h`. The `Library` is stored
        // in `DynamicApi`, so every copied pointer remains valid.
        unsafe {
            Ok(Self {
                create_context: load(&library, b"krun_create_ctx\0")?,
                free_context: load(&library, b"krun_free_ctx\0")?,
                set_vm_config: load(&library, b"krun_set_vm_config\0")?,
                set_root: load(&library, b"krun_set_root\0")?,
                add_disk: load(&library, b"krun_add_disk3\0")?,
                set_root_disk_remount: load(&library, b"krun_set_root_disk_remount\0")?,
                add_virtio_fs: load(&library, b"krun_add_virtiofs3\0")?,
                add_net_unixstream: load(&library, b"krun_add_net_unixstream\0")?,
                disable_implicit_vsock: load(&library, b"krun_disable_implicit_vsock\0")?,
                add_vsock: load(&library, b"krun_add_vsock\0")?,
                add_vsock_port: load(&library, b"krun_add_vsock_port\0")?,
                set_exec: load(&library, b"krun_set_exec\0")?,
                start_enter: load(&library, b"krun_start_enter\0")?,
                _library: library,
            })
        }
    }
}

impl KrunApi for DynamicApi {
    fn create_context(&self) -> i32 {
        // SAFETY: the loaded symbol has the stable no-argument C signature.
        unsafe { (self.create_context)() }
    }

    fn free_context(&self, id: u32) -> i32 {
        // SAFETY: Context owns `id` and calls this at most once.
        unsafe { (self.free_context)(id) }
    }

    fn set_vm_config(&self, id: u32, vcpus: u8, memory_mib: u32) -> i32 {
        // SAFETY: scalar arguments match the stable C declaration.
        unsafe { (self.set_vm_config)(id, vcpus, memory_mib) }
    }

    fn set_root(&self, id: u32, path: &CStr) -> i32 {
        // SAFETY: `path` remains live and NUL-terminated for the call.
        unsafe { (self.set_root)(id, path.as_ptr()) }
    }

    fn add_disk(&self, id: u32, block_id: &CStr, path: &CStr, read_only: bool) -> i32 {
        // SAFETY: both strings remain live and RAW/FULL are stable constants.
        unsafe {
            (self.add_disk)(
                id,
                block_id.as_ptr(),
                path.as_ptr(),
                DISK_FORMAT_RAW,
                read_only,
                false,
                SYNC_FULL,
            )
        }
    }

    fn set_root_disk_remount(&self, id: u32, device: &CStr, filesystem: &CStr) -> i32 {
        // SAFETY: both strings remain live; a null options pointer is allowed.
        unsafe {
            (self.set_root_disk_remount)(id, device.as_ptr(), filesystem.as_ptr(), std::ptr::null())
        }
    }

    fn add_virtio_fs(&self, id: u32, tag: &CStr, path: &CStr, read_only: bool) -> i32 {
        // SAFETY: strings remain live and a zero DAX window is supported.
        unsafe { (self.add_virtio_fs)(id, tag.as_ptr(), path.as_ptr(), 0, read_only) }
    }

    fn add_net_unixstream(&self, id: u32, path: &CStr, mac: &mut [u8; 6]) -> i32 {
        // SAFETY: the path and MAC buffer remain live for the call.
        unsafe {
            (self.add_net_unixstream)(
                id,
                path.as_ptr(),
                -1,
                mac.as_mut_ptr(),
                COMPAT_NET_FEATURES,
                NET_FLAG_DHCP_CLIENT,
            )
        }
    }

    fn disable_implicit_vsock(&self, id: u32) -> i32 {
        // SAFETY: `id` names the live context owned by Context.
        unsafe { (self.disable_implicit_vsock)(id) }
    }

    fn add_vsock(&self, id: u32, cid: u32) -> i32 {
        // SAFETY: both scalar arguments match the stable C declaration.
        unsafe { (self.add_vsock)(id, cid) }
    }

    fn add_vsock_port(&self, id: u32, port: u32, path: &CStr) -> i32 {
        // SAFETY: `path` remains live and NUL-terminated for the call.
        unsafe { (self.add_vsock_port)(id, port, path.as_ptr()) }
    }

    fn set_exec(&self, id: u32, executable: &CStr) -> i32 {
        let empty: [*const std::ffi::c_char; 1] = [std::ptr::null()];
        // SAFETY: executable remains live and both arrays are NULL-terminated.
        unsafe { (self.set_exec)(id, executable.as_ptr(), empty.as_ptr(), empty.as_ptr()) }
    }

    fn start_enter(&self, id: u32) -> i32 {
        // SAFETY: Context consumes the live context before this call.
        unsafe { (self.start_enter)(id) }
    }
}

unsafe fn load<T: Copy>(library: &Library, name: &[u8]) -> Result<T, FfiError> {
    // SAFETY: the caller supplies the exact stable C signature for `name`, and
    // copies the pointer while keeping `library` alive in `DynamicApi`.
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| {
            FfiError::message(
                "resolve libkrun symbol",
                format!(
                    "{}: {error}",
                    String::from_utf8_lossy(name).trim_end_matches('\0')
                ),
            )
        })
}

fn path_cstring(path: &Path) -> Result<CString, FfiError> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| FfiError::message("convert path", "path contains NUL"))
}

const fn status(operation: &'static str, result: i32) -> Result<(), FfiError> {
    if result < 0 {
        Err(FfiError::code(operation, result))
    } else {
        Ok(())
    }
}

fn deterministic_mac(id: &str) -> [u8; 6] {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let bytes = hash.to_be_bytes();
    [0x5a, 0x94, bytes[4], bytes[5], bytes[6], bytes[7]]
}

/// Error returned at the safe libkrun boundary.
#[derive(Debug)]
pub struct FfiError {
    operation: &'static str,
    detail: FfiErrorDetail,
}

#[derive(Debug)]
enum FfiErrorDetail {
    Code(i32),
    Message(String),
}

impl FfiError {
    const fn code(operation: &'static str, code: i32) -> Self {
        Self {
            operation,
            detail: FfiErrorDetail::Code(code),
        }
    }

    fn message(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            detail: FfiErrorDetail::Message(message.into()),
        }
    }

    pub fn diagnostic_code(&self) -> String {
        match self.detail {
            FfiErrorDetail::Code(code) => format!("libkrun-errno-{}", code.unsigned_abs()),
            FfiErrorDetail::Message(_) => "libkrun-api".to_owned(),
        }
    }
}

impl fmt::Display for FfiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            FfiErrorDetail::Code(code) => {
                write!(formatter, "{} failed with status {code}", self.operation)
            }
            FfiErrorDetail::Message(message) => {
                write!(formatter, "{} failed: {message}", self.operation)
            }
        }
    }
}

impl std::error::Error for FfiError {}

#[cfg(test)]
mod tests {
    use super::{Context, KrunApi, deterministic_mac, path_cstring, status};
    use crate::validation::{
        PreparedCommand, PreparedDisk, PreparedForward, PreparedMount, PreparedNetwork,
        PreparedRoot, PreparedSpec,
    };
    use std::{
        collections::BTreeMap,
        ffi::CStr,
        net::{IpAddr, Ipv4Addr},
        path::Path,
        sync::{Arc, Mutex, MutexGuard},
    };

    #[test]
    fn negative_status_becomes_typed_error() {
        let error = status("test", -22).unwrap_err();
        assert_eq!(error.diagnostic_code(), "libkrun-errno-22");
        assert!(error.to_string().contains("-22"));
    }

    #[test]
    fn ffi_paths_reject_interior_nul() {
        assert!(path_cstring(Path::new("bad\0path")).is_err());
    }

    #[test]
    fn generated_mac_is_local_and_stable() {
        let first = deterministic_mac("agent-1");
        assert_eq!(first, deterministic_mac("agent-1"));
        assert_eq!(first[0] & 0b11, 0b10);
    }

    #[test]
    fn safe_configuration_maps_every_device_to_expected_api_call() {
        let api = Arc::new(RecordingApi::new(None));
        let context = Context::from_api(api.clone()).unwrap();
        context
            .configure(
                &prepared_spec(),
                Some(Path::new("/run/vm/passt.sock")),
                Path::new("/run/vm/control.sock"),
                None,
            )
            .unwrap();
        drop(context);

        assert_eq!(
            api.calls(),
            vec![
                Call::Create,
                Call::VmConfig {
                    id: 7,
                    vcpus: 2,
                    memory_mib: 1024,
                },
                Call::Root {
                    id: 7,
                    path: String::from("/images/root"),
                },
                Call::Disk {
                    id: 7,
                    block_id: String::from("sqlite"),
                    path: String::from("/disks/sqlite.raw"),
                    read_only: false,
                },
                Call::VirtioFs {
                    id: 7,
                    tag: String::from("workspace"),
                    path: String::from("/mounts/workspace"),
                    read_only: false,
                },
                Call::DisableImplicitVsock { id: 7 },
                Call::Vsock { id: 7, cid: 0 },
                Call::VsockPort {
                    id: 7,
                    port: crate::protocol::GUEST_VSOCK_PORT,
                    path: String::from("/run/vm/control.sock"),
                },
                Call::Network {
                    id: 7,
                    path: String::from("/run/vm/passt.sock"),
                    mac: deterministic_mac("recording"),
                },
                Call::Exec {
                    id: 7,
                    executable: String::from("/usr/libexec/hephaestus/heph-init"),
                },
                Call::Free { id: 7 },
            ]
        );
    }

    #[test]
    fn raw_root_uses_explicit_raw_disk_and_remount_calls() {
        let api = Arc::new(RecordingApi::new(None));
        let context = Context::from_api(api.clone()).unwrap();
        let mut spec = prepared_spec();
        spec.root = PreparedRoot::RawDisk {
            path: Path::new("/images/root.raw").to_path_buf(),
            read_only: true,
        };
        spec.disks.clear();
        spec.mounts.clear();
        spec.network = PreparedNetwork::Disabled;
        context
            .configure(&spec, None, Path::new("/run/vm/control.sock"), None)
            .unwrap();
        drop(context);
        let calls = api.calls();
        assert!(calls.contains(&Call::Disk {
            id: 7,
            block_id: String::from("root"),
            path: String::from("/images/root.raw"),
            read_only: true,
        }));
        assert!(calls.contains(&Call::RootRemount {
            id: 7,
            device: String::from("/dev/vda"),
            filesystem: String::from("auto"),
        }));
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, Call::Network { .. }))
        );
    }

    #[test]
    fn broker_only_adds_dedicated_vsock_without_ip_network() {
        let api = Arc::new(RecordingApi::new(None));
        let context = Context::from_api(api.clone()).unwrap();
        let mut spec = prepared_spec();
        spec.network = PreparedNetwork::BrokerOnly;
        context
            .configure(
                &spec,
                None,
                Path::new("/run/vm/control.sock"),
                Some(Path::new("/run/hephaestus/broker.sock")),
            )
            .unwrap();
        drop(context);
        let calls = api.calls();
        assert!(calls.contains(&Call::VsockPort {
            id: 7,
            port: crate::protocol::SECRET_BROKER_VSOCK_PORT,
            path: String::from("/run/hephaestus/broker.sock"),
        }));
        assert!(
            calls
                .iter()
                .all(|call| !matches!(call, Call::Network { .. }))
        );
    }

    #[test]
    fn every_injected_api_failure_retains_operation_and_code() {
        for operation in [
            "create",
            "vm-config",
            "root",
            "disk",
            "virtio-fs",
            "disable-vsock",
            "vsock",
            "vsock-port",
            "network",
            "exec",
        ] {
            let api = Arc::new(RecordingApi::new(Some(operation)));
            let result = Context::from_api(api.clone()).and_then(|context| {
                context.configure(
                    &prepared_spec(),
                    Some(Path::new("/run/vm/passt.sock")),
                    Path::new("/run/vm/control.sock"),
                    None,
                )
            });
            let error = result.expect_err("injected FFI failure must propagate");
            assert_eq!(error.diagnostic_code(), "libkrun-errno-22");
            assert!(error.to_string().contains("-22"));
            if operation != "create" {
                assert_eq!(api.calls().last(), Some(&Call::Free { id: 7 }));
            }
        }

        let remount_api = Arc::new(RecordingApi::new(Some("root-remount")));
        let remount_context = Context::from_api(remount_api).unwrap();
        let mut raw_root = prepared_spec();
        raw_root.root = PreparedRoot::RawDisk {
            path: Path::new("/images/root.raw").to_path_buf(),
            read_only: true,
        };
        assert_eq!(
            remount_context
                .configure(
                    &raw_root,
                    Some(Path::new("/run/vm/passt.sock")),
                    Path::new("/run/vm/control.sock"),
                    None,
                )
                .unwrap_err()
                .diagnostic_code(),
            "libkrun-errno-22"
        );

        let start_api = Arc::new(RecordingApi::new(Some("start")));
        let start_context = Context::from_api(start_api).unwrap();
        assert_eq!(
            start_context.start_enter().unwrap_err().diagnostic_code(),
            "libkrun-errno-22"
        );
    }

    fn prepared_spec() -> PreparedSpec {
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
            labels: BTreeMap::new(),
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
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

    struct RecordingApi {
        calls: Mutex<Vec<Call>>,
        fail_operation: Option<&'static str>,
    }

    impl RecordingApi {
        const fn new(fail_operation: Option<&'static str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_operation,
            }
        }

        fn calls(&self) -> Vec<Call> {
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
}
