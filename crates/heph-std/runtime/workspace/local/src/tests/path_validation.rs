use super::super::*;
use crate::{
    import::declared_regular_file,
    safety::{validate_relative_path, validate_symlink_target},
    validation::validate_message,
};
use std::{fs, os::unix::fs::symlink};

#[test]
fn rejects_repository_and_symlink_path_escapes() {
    for path in [
        "../escape",
        "/absolute",
        ".git/config",
        "nested/../../escape",
    ] {
        assert!(matches!(
            validate_relative_path(path),
            Err(LocalWorkspaceError::UnsafePath(_))
        ));
    }
    for target in ["../escape", "/absolute", "nested/../escape", "."] {
        assert!(matches!(
            validate_symlink_target(target),
            Err(LocalWorkspaceError::UnsafePath(_))
        ));
    }
    validate_relative_path("reports/result.json").expect("safe repository path");
    validate_symlink_target("reports/result.json").expect("safe symlink target");
}

#[test]
fn validates_result_message_bounds() {
    assert_eq!(
        validate_message("   ").expect("default result message"),
        "Hephaestus agent result"
    );
    assert!(matches!(
        validate_message(&"x".repeat(4097)),
        Err(LocalWorkspaceError::InvalidResult(_))
    ));
    assert!(matches!(
        validate_message("bad\0message"),
        Err(LocalWorkspaceError::InvalidResult(_))
    ));
}

#[test]
fn declared_file_lookup_never_traverses_a_symlink() {
    let temporary = tempfile::tempdir().expect("temporary declared file");
    let work = temporary.path().join("work");
    fs::create_dir(&work).expect("work directory");
    let actual = work.join("actual");
    fs::create_dir(&actual).expect("actual directory");
    fs::write(actual.join("result.txt"), b"result").expect("actual result");
    symlink("actual", work.join("linked")).expect("linked directory");
    assert!(matches!(
        declared_regular_file(&work, "linked/result.txt"),
        Err(LocalWorkspaceError::InvalidResult(_))
    ));
    declared_regular_file(&work, "actual/result.txt").expect("direct regular file");
}
