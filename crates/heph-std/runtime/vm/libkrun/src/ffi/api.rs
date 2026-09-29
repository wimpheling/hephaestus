use libloading::Library;
use std::ffi::{CStr, OsStr};

use crate::ffi::errors::FfiError;

pub const DISK_FORMAT_RAW: u32 = 0;
pub const SYNC_FULL: u32 = 2;
pub const NET_FLAG_DHCP_CLIENT: u32 = 1 << 1;
pub const COMPAT_NET_FEATURES: u32 =
    (1 << 0) | (1 << 1) | (1 << 7) | (1 << 10) | (1 << 11) | (1 << 14);
pub const ROOT_BLOCK_ID: &CStr = c"root";
pub const ROOT_DEVICE: &CStr = c"/dev/vda";
pub const AUTO_FS: &CStr = c"auto";
pub const HEPH_INIT: &CStr = c"/usr/libexec/hephaestus/heph-init";

#[allow(unsafe_code)]
type CreateContext = unsafe extern "C" fn() -> i32;
#[allow(unsafe_code)]
type FreeContext = unsafe extern "C" fn(u32) -> i32;
#[allow(unsafe_code)]
type SetVmConfig = unsafe extern "C" fn(u32, u8, u32) -> i32;
#[allow(unsafe_code)]
type SetRoot = unsafe extern "C" fn(u32, *const std::ffi::c_char) -> i32;
#[allow(unsafe_code)]
type AddDisk = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
    u32,
    bool,
    bool,
    u32,
) -> i32;
#[allow(unsafe_code)]
type SetRootDiskRemount = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
    *const std::ffi::c_char,
) -> i32;
#[allow(unsafe_code)]
type AddVirtioFs =
    unsafe extern "C" fn(u32, *const std::ffi::c_char, *const std::ffi::c_char, u64, bool) -> i32;
#[allow(unsafe_code)]
type AddNetUnixStream =
    unsafe extern "C" fn(u32, *const std::ffi::c_char, i32, *mut u8, u32, u32) -> i32;
#[allow(unsafe_code)]
type DisableImplicitVsock = unsafe extern "C" fn(u32) -> i32;
#[allow(unsafe_code)]
type AddVsock = unsafe extern "C" fn(u32, u32) -> i32;
#[allow(unsafe_code)]
type AddVsockPort = unsafe extern "C" fn(u32, u32, *const std::ffi::c_char) -> i32;
#[allow(unsafe_code)]
type SetExec = unsafe extern "C" fn(
    u32,
    *const std::ffi::c_char,
    *const *const std::ffi::c_char,
    *const *const std::ffi::c_char,
) -> i32;
#[allow(unsafe_code)]
type StartEnter = unsafe extern "C" fn(u32) -> i32;

pub struct DynamicApi {
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

pub trait KrunApi: Send + Sync {
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

impl DynamicApi {
    // Loading administrator-selected libkrun symbols is the dedicated FFI trust boundary.
    #[allow(unsafe_code)]
    pub fn load(path: &OsStr) -> Result<Self, FfiError> {
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

// Each method forwards validated Rust values to one stable libkrun C ABI call.
#[allow(unsafe_code)]
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

#[allow(unsafe_code)]
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
