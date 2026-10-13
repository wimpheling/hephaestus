//! Check guest path spelling before provider entry; never normalize or rewrite it.
use std::path::Path;

pub(super) fn allowed(working_dir: Option<&Path>) -> bool {
    let Some(path) = working_dir else {
        return true;
    };
    let Some(path) = path.to_str() else {
        return false;
    };
    if path.chars().any(char::is_control) {
        return false;
    }
    path == "/release"
        || path.strip_prefix("/release/").is_some_and(|relative| {
            relative
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
        })
}

#[cfg(test)]
mod tests {
    use super::allowed;
    use std::path::Path;

    #[test]
    fn exact_release_root_and_descendants_are_valid() {
        assert!(allowed(None));
        for path in [
            "/release",
            "/release/bin",
            "/release/app data/子",
            "/release/.config",
        ] {
            assert!(allowed(Some(Path::new(path))), "{path:?}");
        }
    }

    #[test]
    fn outside_release_and_lexical_aliases_are_invalid() {
        for path in [
            "",
            "/",
            "release",
            "/workspace",
            "/tmp/release",
            "/releaseX",
            "/releaseX/bin",
            "//release",
            "/release/",
            "/release//bin",
            "/release/bin/",
            "/release/.",
            "/release/./bin",
            "/release/..",
            "/release/../workspace",
            "/release/bin/../../workspace",
            "/release/\0",
            "/release/\n",
            "/release/\t",
            "/release/\u{7f}",
            "/release/\u{85}",
        ] {
            assert!(!allowed(Some(Path::new(path))), "{path:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_guest_path_is_invalid() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let path = Path::new(OsStr::from_bytes(b"/release/\xff"));
        assert!(!allowed(Some(path)));
    }
}
