use std::{ffi::CString, fmt, os::unix::ffi::OsStrExt, path::Path};

pub fn path_cstring(path: &Path) -> Result<CString, FfiError> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| FfiError::message("convert path", "path contains NUL"))
}

pub const fn status(operation: &'static str, result: i32) -> Result<(), FfiError> {
    if result < 0 {
        Err(FfiError::code(operation, result))
    } else {
        Ok(())
    }
}

pub fn deterministic_mac(id: &str) -> [u8; 6] {
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
    pub const fn code(operation: &'static str, code: i32) -> Self {
        Self {
            operation,
            detail: FfiErrorDetail::Code(code),
        }
    }

    pub fn message(operation: &'static str, message: impl Into<String>) -> Self {
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
