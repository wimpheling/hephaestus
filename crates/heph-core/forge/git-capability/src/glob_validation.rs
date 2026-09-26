use crate::{GitCapabilityError, MAX_PATH_GLOB_BYTES, MAX_REF_GLOB_BYTES};

// Git reserves the lowercase `.lock` suffix specifically; this is not a host
// filesystem extension comparison.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub fn validate_ref_glob(value: &str) -> Result<(), GitCapabilityError> {
    validate_common(value, MAX_REF_GLOB_BYTES).map_err(GitCapabilityError::InvalidRefGlob)?;
    let segments: Vec<_> = value.split('/').collect();
    if segments.len() < 3 || segments[0] != "refs" || !is_literal_segment(segments[1]) {
        return Err(GitCapabilityError::InvalidRefGlob(
            "must be anchored below an explicit refs/<namespace>/ prefix",
        ));
    }
    if value.contains("@{")
        || segments.iter().any(|segment| {
            segment.ends_with('.')
                || segment.ends_with(".lock")
                || segment.starts_with('.')
                || segment
                    .chars()
                    .any(|character| matches!(character, ' ' | '~' | '^' | ':' | '?' | '['))
        })
    {
        return Err(GitCapabilityError::InvalidRefGlob(
            "contains a Git-ref-forbidden spelling",
        ));
    }
    Ok(())
}

pub fn validate_path_glob(value: &str) -> Result<(), GitCapabilityError> {
    validate_common(value, MAX_PATH_GLOB_BYTES).map_err(GitCapabilityError::InvalidChangedPathGlob)
}

pub fn validate_common(value: &str, max_bytes: usize) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > max_bytes {
        return Err("is empty or exceeds its byte bound");
    }
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
        return Err("must be anchored and contain no empty segments");
    }
    if value
        .chars()
        .any(|character| character == '\\' || character.is_control())
    {
        return Err("contains a backslash or control character");
    }
    for segment in value.split('/') {
        if segment == "." || segment == ".." {
            return Err("contains a dot path segment");
        }
        if segment.contains("**") && segment != "**" {
            return Err("uses ** inside a segment");
        }
    }
    Ok(())
}

pub fn is_literal_segment(segment: &str) -> bool {
    !segment.contains('*')
}

pub fn is_broad_ref_glob(value: &str) -> bool {
    let mut segments = value.split('/');
    let _refs = segments.next();
    let _namespace = segments.next();
    matches!((segments.next(), segments.next()), (Some("*" | "**"), None))
}

pub fn validate_concrete_ref(value: &str) -> Result<(), GitCapabilityError> {
    validate_ref_glob(value)?;
    if value.contains('*') {
        return Err(GitCapabilityError::InvalidRefGlob(
            "a concrete ref cannot contain wildcards",
        ));
    }
    Ok(())
}

pub fn validate_concrete_path(value: &str) -> Result<(), GitCapabilityError> {
    validate_path_glob(value)?;
    if value.contains('*') {
        return Err(GitCapabilityError::InvalidChangedPathGlob(
            "a concrete path cannot contain wildcards",
        ));
    }
    Ok(())
}

pub fn glob_matches(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<_> = pattern.split('/').collect();
    let candidate: Vec<_> = candidate.split('/').collect();
    let mut reachable = vec![false; candidate.len() + 1];
    reachable[0] = true;
    for pattern_segment in pattern {
        if pattern_segment == "**" {
            for index in 1..=candidate.len() {
                reachable[index] = reachable[index] || reachable[index - 1];
            }
        } else {
            let mut next = vec![false; candidate.len() + 1];
            for index in 1..=candidate.len() {
                next[index] =
                    reachable[index - 1] && segment_matches(pattern_segment, candidate[index - 1]);
            }
            reachable = next;
        }
    }
    reachable[candidate.len()]
}

pub fn segment_matches(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<_> = pattern.chars().collect();
    let candidate: Vec<_> = candidate.chars().collect();
    let mut reachable = vec![false; candidate.len() + 1];
    reachable[0] = true;
    for character in pattern {
        if character == '*' {
            for index in 1..=candidate.len() {
                reachable[index] = reachable[index] || reachable[index - 1];
            }
        } else {
            for index in (1..=candidate.len()).rev() {
                reachable[index] = reachable[index - 1] && candidate[index - 1] == character;
            }
            reachable[0] = false;
        }
    }
    reachable[candidate.len()]
}
