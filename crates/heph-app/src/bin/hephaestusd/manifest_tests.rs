use super::root_images::{
    legacy_fixture_root_images, load_root_image_manifest, repository_root_images,
};
use std::{ffi::OsString, fs::File};
use tempfile::tempdir;
use vm_trait::{DiskFormat, RootFilesystem};

#[test]
fn manifest_loads_multiple_materialized_directory_roots() {
    let temporary = tempdir().expect("temporary root");
    let first = temporary.path().join("ubuntu");
    let second = temporary.path().join("rust");
    std::fs::create_dir(&first).expect("first root");
    std::fs::create_dir(&second).expect("second root");
    let manifest = format!(
        r#"{{"version":1,"roots":{{
        "ubuntu@sha256:{}":{{"kind":"directory","path":"{}"}},
        "rust@sha256:{}":{{"kind":"directory","path":"{}"}}
    }}}}"#,
        "a".repeat(64),
        first.display(),
        "b".repeat(64),
        second.display(),
    );
    let manifest_path = temporary.path().join("roots.json");
    std::fs::write(&manifest_path, manifest).expect("manifest");

    let roots = load_root_image_manifest(&manifest_path).expect("valid manifest");
    assert_eq!(roots.len(), 2);
    assert!(roots.contains_key(&format!("ubuntu@sha256:{}", "a".repeat(64))));
    assert!(roots.contains_key(&format!("rust@sha256:{}", "b".repeat(64))));
}

#[test]
fn manifest_rejects_unpinned_references_and_missing_materialization() {
    let temporary = tempdir().expect("temporary root");
    let manifest_path = temporary.path().join("roots.json");
    std::fs::write(
        &manifest_path,
        format!(
            r#"{{"version":1,"roots":{{"ubuntu:latest":{{"kind":"directory","path":"{}"}}}}}}"#,
            temporary.path().display()
        ),
    )
    .expect("manifest");
    assert!(load_root_image_manifest(&manifest_path).is_err());

    std::fs::write(
        &manifest_path,
        format!(
            r#"{{"version":1,"roots":{{"ubuntu@sha256:{}":{{"kind":"directory","path":"{}"}}}}}}"#,
            "a".repeat(64),
            temporary.path().join("missing").display()
        ),
    )
    .expect("manifest");
    assert!(load_root_image_manifest(&manifest_path).is_err());
}

#[test]
fn manifest_supports_explicit_read_only_disk_roots() {
    let temporary = tempdir().expect("temporary root");
    let disk = temporary.path().join("ubuntu.raw");
    File::create(&disk).expect("disk");
    let manifest_path = temporary.path().join("roots.json");
    std::fs::write(
    &manifest_path,
    format!(
        r#"{{"version":1,"roots":{{"ubuntu@sha256:{}":{{"kind":"disk","path":"{}","format":"raw","read_only":true}}}}}}"#,
        "a".repeat(64),
        disk.display()
    ),
)
.expect("manifest");

    let roots = load_root_image_manifest(&manifest_path).expect("valid disk manifest");
    assert!(matches!(
        roots.values().next(),
        Some(RootFilesystem::Disk {
            format: DiskFormat::Raw,
            read_only: true,
            ..
        })
    ));
}

#[test]
fn repository_manifest_loads_only_worker_materialized_directories() {
    let temporary = tempdir().expect("temporary root");
    let rootfs = temporary.path().join("repository-rootfs");
    let image = rootfs.join("sha256-aaaaaaaa");
    std::fs::create_dir_all(&image).expect("materialized image root");
    let manifest_path = temporary.path().join("repository-roots.json");
    std::fs::write(
    &manifest_path,
    format!(
        r#"{{"version":1,"roots":{{"registry.example/project/image@sha256:{}":{{"kind":"directory","path":"{}"}}}}}}"#,
        "a".repeat(64),
        image.display()
    ),
)
.expect("manifest");

    let image_roots = repository_root_images(&manifest_path, &rootfs).expect("trusted root");
    assert_eq!(image_roots.len(), 1);

    std::fs::write(
    &manifest_path,
    format!(
        r#"{{"version":1,"roots":{{"registry.example/project/image@sha256:{}":{{"kind":"directory","path":"{}"}}}}}}"#,
        "a".repeat(64),
        temporary.path().display()
    ),
)
.expect("outside manifest");
    assert!(repository_root_images(&manifest_path, &rootfs).is_err());
}

#[test]
fn missing_repository_manifest_means_no_project_roots_yet() {
    let temporary = tempdir().expect("temporary root");
    let rootfs = temporary.path().join("repository-rootfs");
    std::fs::create_dir(&rootfs).expect("rootfs root");
    let image_roots = repository_root_images(&temporary.path().join("missing.json"), &rootfs)
        .expect("empty initial state");
    assert!(image_roots.is_empty());
}

#[test]
fn legacy_fixture_pair_still_resolves_to_a_directory_root() {
    let temporary = tempdir().expect("temporary root");
    let roots = legacy_fixture_root_images(
        temporary.path().to_owned(),
        OsString::from(format!("fixture@sha256:{}", "a".repeat(64))),
    )
    .expect("legacy fixture root");
    assert!(matches!(
        roots.values().next(),
        Some(RootFilesystem::Directory { host_path }) if host_path == temporary.path()
    ));
}
