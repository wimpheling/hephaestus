use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};

use super::{
    ArchitectureConfiguration, ArchitectureException, Diagnostic, ExceptionSelector,
    architecture_registry::known_rule_ids, db_architecture,
};

pub(super) fn usable_exceptions<'a>(
    root: &Path,
    configuration: &'a ArchitectureConfiguration,
) -> Vec<&'a ArchitectureException> {
    let mut seen = BTreeSet::new();
    configuration
        .exceptions
        .iter()
        .filter(|exception| {
            known_rule_ids().contains(exception.rule_id.as_str())
                && !exception.rationale.trim().is_empty()
                && !exception.owner.trim().is_empty()
                && (exception
                    .expires
                    .as_deref()
                    .is_some_and(|expiry| !expiry.trim().is_empty())
                    || exception
                        .tracking_task
                        .as_deref()
                        .is_some_and(|task| !task.trim().is_empty()))
        })
        .filter(|exception| {
            let expiry_valid = exception
                .expires
                .as_deref()
                .is_none_or(|expiry| date_shape(expiry) && !expiry_has_passed(expiry));
            let tracking_valid = exception
                .tracking_task
                .as_deref()
                .is_none_or(|task| validate_tracking_task(root, task).is_ok());
            let scope_valid = if exception.rule_id == "DB-STATIC-SQL" {
                validate_exact_scope_shape(root, &exception.scope).is_ok()
            } else {
                validate_exact_scope(root, &exception.scope).is_ok()
            };
            expiry_valid
                && tracking_valid
                && scope_valid
                && (exception.rule_id != "DB-STATIC-SQL"
                    || db_architecture::validate_exception_scope(root, &exception.scope).is_ok())
                && seen.insert((&exception.rule_id, &exception.scope))
        })
        .filter(|exception| exception.rule_id == "DB-STATIC-SQL")
        .collect()
}
pub(super) fn validate_exceptions(
    root: &Path,
    configuration: &ArchitectureConfiguration,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    for exception in &configuration.exceptions {
        if !known_rule_ids().contains(exception.rule_id.as_str()) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("exception names unknown rule {}", exception.rule_id),
            ));
        }
        if exception.rationale.trim().is_empty() || exception.owner.trim().is_empty() {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} requires non-empty rationale and owner",
                    exception.rule_id
                ),
            ));
        }
        let has_expiry = exception
            .expires
            .as_deref()
            .is_some_and(|expiry| !expiry.trim().is_empty());
        let has_tracking_task = exception
            .tracking_task
            .as_deref()
            .is_some_and(|task| !task.trim().is_empty());
        if !has_expiry && !has_tracking_task {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} requires an expiry or tracking task",
                    exception.rule_id
                ),
            ));
        }
        if exception
            .expires
            .as_deref()
            .is_some_and(|date| !date_shape(date))
        {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} has invalid YYYY-MM-DD expiry",
                    exception.rule_id
                ),
            ));
        }
        if let Some(task) = exception
            .tracking_task
            .as_deref()
            .filter(|task| !task.trim().is_empty())
        {
            if let Err(reason) = validate_tracking_task(root, task) {
                diagnostics.push(Diagnostic::new(
                    "ARCH-EXCEPTION-FORMAT",
                    format!(
                        "exception for {} has unresolved tracking task `{task}`: {reason}",
                        exception.rule_id
                    ),
                ));
            }
        }
        if exception.expires.as_deref().is_some_and(expiry_has_passed) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!("exception for {} has expired", exception.rule_id),
            ));
        }
        if !seen.insert((&exception.rule_id, &exception.scope)) {
            diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} duplicates exact scope `{}`",
                    exception.rule_id, exception.scope
                ),
            ));
        }
        let scope_validation = if exception.rule_id == "DB-STATIC-SQL" {
            validate_exact_scope_shape(root, &exception.scope)
        } else {
            validate_exact_scope(root, &exception.scope)
        };
        match scope_validation {
            Err(reason) => diagnostics.push(Diagnostic::new(
                "ARCH-EXCEPTION-FORMAT",
                format!(
                    "exception for {} has invalid scope `{}`: {reason}",
                    exception.rule_id, exception.scope
                ),
            )),
            Ok(()) => validate_db_exception_scope(root, exception, diagnostics),
        }
    }
}

pub(super) fn validate_db_exception_scope(
    root: &Path,
    exception: &ArchitectureException,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if exception.rule_id != "DB-STATIC-SQL" {
        return;
    }
    if let Err(reason) = db_architecture::validate_exception_scope(root, &exception.scope) {
        diagnostics.push(Diagnostic::new(
            "ARCH-EXCEPTION-FORMAT",
            format!(
                "exception for {} has invalid Rust item scope `{}`: {reason}",
                exception.rule_id, exception.scope
            ),
        ));
    }
}

pub(super) fn validate_tracking_task(
    root: &Path,
    task: &str,
) -> std::result::Result<(), &'static str> {
    let relative = Path::new(task);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err("use a repository-relative path without parent traversal");
    }
    if !root.join(relative).is_file() {
        return Err("task file does not exist");
    }
    Ok(())
}

pub(super) fn validate_exact_scope(
    root: &Path,
    scope: &str,
) -> std::result::Result<(), &'static str> {
    if scope.contains(['*', '?', '[', ']']) {
        return Err("globs are forbidden");
    }

    let (path_text, selector) = if let Some((path, item)) = scope.split_once('#') {
        if item.trim().is_empty() {
            return Err("the item selector is empty");
        }
        (path, ExceptionSelector::Item(item))
    } else if let Some((path, line)) = scope.rsplit_once(':') {
        let Ok(line) = line.parse::<usize>() else {
            return Err("the line selector is invalid");
        };
        if line == 0 {
            return Err("the line selector is invalid");
        }
        (path, ExceptionSelector::Line(line))
    } else {
        return Err("use path:line or path#item");
    };

    let relative = Path::new(path_text);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err("scope must be a repository-relative path without parent traversal");
    }
    let target = root.join(relative);
    if target.is_dir() {
        return Err("directory-wide exceptions are forbidden");
    }
    if !target.is_file() {
        return Err("scoped file does not exist");
    }
    let source = fs::read_to_string(target).map_err(|_| "scoped file is not readable text")?;
    match selector {
        ExceptionSelector::Item(item) if !source.contains(item) => {
            return Err("scoped item does not exist in the file");
        }
        ExceptionSelector::Line(line) if source.lines().count() < line => {
            return Err("scoped line is beyond the end of the file");
        }
        ExceptionSelector::Item(_) | ExceptionSelector::Line(_) => {}
    }
    Ok(())
}

/// Validates file and selector shape while leaving Rust item resolution to the
/// DB checker, where module and type qualification can be parsed structurally.
pub(super) fn validate_exact_scope_shape(
    root: &Path,
    scope: &str,
) -> std::result::Result<(), &'static str> {
    if scope.contains(['*', '?', '[', ']']) {
        return Err("globs are forbidden");
    }
    let (path_text, selector) = if let Some((path, item)) = scope.split_once('#') {
        if item.trim().is_empty() {
            return Err("the item selector is empty");
        }
        (path, ExceptionSelector::Item(item))
    } else if let Some((path, line)) = scope.rsplit_once(':') {
        let Ok(line) = line.parse::<usize>() else {
            return Err("the line selector is invalid");
        };
        if line == 0 {
            return Err("the line selector is invalid");
        }
        (path, ExceptionSelector::Line(line))
    } else {
        return Err("use path:line or path#item");
    };
    let relative = Path::new(path_text);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err("scope must be a repository-relative path without parent traversal");
    }
    let target = root.join(relative);
    if target.is_dir() {
        return Err("directory-wide exceptions are forbidden");
    }
    let source = fs::read_to_string(target).map_err(|_| "scoped file is not readable text")?;
    if let ExceptionSelector::Line(line) = selector {
        if source.lines().count() < line {
            return Err("scoped line is beyond the end of the file");
        }
    }
    Ok(())
}
pub(super) fn date_shape(date: &str) -> bool {
    let bytes = date.as_bytes();
    if !(bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()))
    {
        return false;
    }
    let Ok(year) = date[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = date[5..7].parse::<usize>() else {
        return false;
    };
    let Ok(day) = date[8..10].parse::<u32>() else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    month_lengths
        .get(month.saturating_sub(1))
        .is_some_and(|maximum| month > 0 && day > 0 && day <= *maximum)
}

pub(super) fn expiry_has_passed(expiry: &str) -> bool {
    date_shape(expiry) && expiry < time::OffsetDateTime::now_utc().date().to_string().as_str()
}
