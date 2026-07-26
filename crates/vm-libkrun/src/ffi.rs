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

struct Api {
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

/// Safe owner for one libkrun configuration context.
pub struct Context {
    api: Arc<Api>,
    id: Option<u32>,
}

impl Context {
    pub fn load(library: &OsStr) -> Result<Self, FfiError> {
        let api = Arc::new(Api::load(library)?);
        // SAFETY: the symbol signature is the stable libkrun 1.x C API and
        // requires no arguments.
        let result = unsafe { (api.create_context)() };
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
    ) -> Result<(), FfiError> {
        let id = self.id();
        status(
            "krun_set_vm_config",
            // SAFETY: `id` owns a live context and the scalar arguments match
            // the stable C declaration.
            unsafe { (self.api.set_vm_config)(id, spec.vcpus, spec.memory_mib) },
        )?;

        match &spec.root {
            PreparedRoot::Directory { path } => {
                let path = path_cstring(path)?;
                status(
                    "krun_set_root",
                    // SAFETY: `path` is NUL-terminated and remains alive for
                    // the duration of the call.
                    unsafe { (self.api.set_root)(id, path.as_ptr()) },
                )?;
            }
            PreparedRoot::RawDisk { path, read_only } => {
                self.add_disk(ROOT_BLOCK_ID, path, *read_only)?;
                status(
                    "krun_set_root_disk_remount",
                    // SAFETY: all string pointers are static C strings and
                    // `id` owns a live context.
                    unsafe {
                        (self.api.set_root_disk_remount)(
                            id,
                            ROOT_DEVICE.as_ptr(),
                            AUTO_FS.as_ptr(),
                            std::ptr::null(),
                        )
                    },
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
            // SAFETY: `id` owns a live context.
            unsafe { (self.api.disable_implicit_vsock)(id) },
        )?;
        status(
            "krun_add_vsock",
            // SAFETY: zero explicitly disables TSI while retaining the
            // control-channel vsock device.
            unsafe { (self.api.add_vsock)(id, 0) },
        )?;
        let control_socket = path_cstring(control_socket)?;
        status(
            "krun_add_vsock_port",
            // SAFETY: the socket path is a live NUL-terminated string.
            unsafe {
                (self.api.add_vsock_port)(
                    id,
                    crate::protocol::GUEST_VSOCK_PORT,
                    control_socket.as_ptr(),
                )
            },
        )?;

        match (&spec.network, passt_socket) {
            (PreparedNetwork::Disabled, None) => {}
            (PreparedNetwork::UserMode { .. }, Some(socket)) => {
                let socket = path_cstring(socket)?;
                let mut mac = deterministic_mac(&spec.id);
                status(
                    "krun_add_net_unixstream",
                    // SAFETY: the socket and MAC pointers remain valid for the
                    // call and match the stable libkrun declaration.
                    unsafe {
                        (self.api.add_net_unixstream)(
                            id,
                            socket.as_ptr(),
                            -1,
                            mac.as_mut_ptr(),
                            COMPAT_NET_FEATURES,
                            NET_FLAG_DHCP_CLIENT,
                        )
                    },
                )?;
            }
            _ => {
                return Err(FfiError::message(
                    "network configuration",
                    "passt socket does not match requested network mode",
                ));
            }
        }

        let empty: [*const std::ffi::c_char; 1] = [std::ptr::null()];
        status(
            "krun_set_exec",
            // SAFETY: the executable is a static C string and both pointer
            // arrays contain only their terminating NULL entries.
            unsafe { (self.api.set_exec)(id, HEPH_INIT.as_ptr(), empty.as_ptr(), empty.as_ptr()) },
        )
    }

    pub fn start_enter(mut self) -> Result<(), FfiError> {
        let id = self.id.take().expect("context is consumed only once");
        // SAFETY: this consumes the live context exactly once. On success
        // libkrun takes over the worker process and does not return.
        let result = unsafe { (self.api.start_enter)(id) };
        status("krun_start_enter", result)
    }

    fn add_disk(&self, block_id: &CStr, path: &Path, read_only: bool) -> Result<(), FfiError> {
        let path = path_cstring(path)?;
        status(
            "krun_add_disk3",
            // SAFETY: both C strings remain alive for the call, and RAW is the
            // explicitly validated image format.
            unsafe {
                (self.api.add_disk)(
                    self.id(),
                    block_id.as_ptr(),
                    path.as_ptr(),
                    DISK_FORMAT_RAW,
                    read_only,
                    false,
                    SYNC_FULL,
                )
            },
        )
    }

    fn add_mount(&self, mount: &PreparedMount) -> Result<(), FfiError> {
        let tag = CString::new(mount.tag.as_bytes())
            .map_err(|_| FfiError::message("krun_add_virtiofs3", "mount tag contains NUL"))?;
        let path = path_cstring(&mount.host_path)?;
        status(
            "krun_add_virtiofs3",
            // SAFETY: both strings remain alive for the call; a zero DAX
            // window requests ordinary virtio-fs operation.
            unsafe {
                (self.api.add_virtio_fs)(self.id(), tag.as_ptr(), path.as_ptr(), 0, mount.read_only)
            },
        )
    }

    const fn id(&self) -> u32 {
        self.id.expect("live context")
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            // SAFETY: `id` is owned by this context and has not been consumed
            // by `krun_start_enter`.
            let _result = unsafe { (self.api.free_context)(id) };
        }
    }
}

impl Api {
    fn load(path: &OsStr) -> Result<Self, FfiError> {
        // SAFETY: loading a configured shared object can run library
        // constructors. The worker is the dedicated trust boundary for
        // libkrun/libkrunfw, and the path is administrator configuration.
        let library = unsafe { Library::new(path) }
            .map_err(|error| FfiError::message("load libkrun", error.to_string()))?;
        // SAFETY: each name and function-pointer type below is copied directly
        // from stable libkrun 1.x's public `libkrun.h`. The `Library` is stored
        // in `Api`, so every copied pointer remains valid.
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

unsafe fn load<T: Copy>(library: &Library, name: &[u8]) -> Result<T, FfiError> {
    // SAFETY: the caller supplies the exact stable C signature for `name`, and
    // copies the pointer while keeping `library` alive in `Api`.
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
    use super::{deterministic_mac, path_cstring, status};
    use std::path::Path;

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
}
