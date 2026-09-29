//! Shared scalar and path validation helpers.
use crate::Diagnostic;
use std::path::Path;

pub fn valid_key(value: &str, maximum: usize) -> bool {
    (1..=maximum).contains(&value.len())
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || ((byte == b'_' || byte == b'-') && index > 0)
        })
}

pub fn validate_relative_path(
    diagnostics: &mut Vec<Diagnostic>,
    path: &str,
    value: &str,
    code: &str,
) {
    if !valid_relative_path(value) {
        diagnostic(
            diagnostics,
            code,
            path,
            "path must be relative, traversal-free, and outside .git",
        );
    }
}

pub fn valid_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value.len() <= 1024
        && !path.is_absolute()
        && !value.contains('\\')
        && path.components().all(|component| {
            matches!(component, std::path::Component::Normal(part)
                if !part.eq_ignore_ascii_case(std::ffi::OsStr::new(".git")))
        })
}

pub fn validate_absolute_path(
    diagnostics: &mut Vec<Diagnostic>,
    path: &str,
    value: &str,
    code: &str,
) {
    let parsed = Path::new(value);
    if !parsed.is_absolute()
        || parsed
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        diagnostic(
            diagnostics,
            code,
            path,
            "path must be absolute and must not contain parent traversal",
        );
    }
}

pub fn diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(Diagnostic {
        code: code.into(),
        path: Some(path.into()),
        message: message.into(),
    });
}
