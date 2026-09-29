//! Focused result-preview security tests.

#[cfg(test)]
mod tests {
    use crate::run::artifact::{PreviewArtifact, load_previews, read_preview};
    use crate::run::model::MAX_RESULT_PREVIEW_BYTES;
    use crate::run::model::RunError;
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    fn artifact(run_id: Uuid, kind: &str, extension: &str, contents: &[u8]) -> PreviewArtifact {
        let sha256 = format!("{:x}", Sha256::digest(contents));
        PreviewArtifact {
            kind: kind.to_owned(),
            size_bytes: i64::try_from(contents.len()).expect("bounded fixture"),
            storage_key: format!("{run_id}/{kind}-{sha256}.{extension}"),
            sha256,
        }
    }

    #[test]
    fn result_previews_are_run_bound_bounded_and_hash_verified() {
        let temporary = tempfile::tempdir().expect("temporary artifact root");
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
            .expect("private artifact root");
        let run_id = Uuid::new_v4();
        let run_directory = temporary.path().join(run_id.to_string());
        fs::create_dir(&run_directory).expect("run artifact directory");
        let patch = b"diff --git a/input.txt b/input.txt\n+changed fixture text\n";
        let manifest = br#"{"entries":[{"path":"input.txt"}]}"#;
        let patch_artifact = artifact(run_id, "patch", "patch", patch);
        let manifest_artifact = artifact(run_id, "manifest", "json", manifest);
        fs::write(temporary.path().join(&patch_artifact.storage_key), patch)
            .expect("patch artifact");
        fs::write(
            temporary.path().join(&manifest_artifact.storage_key),
            manifest,
        )
        .expect("manifest artifact");

        let previews = load_previews(
            temporary.path(),
            run_id,
            &[patch_artifact.clone(), manifest_artifact],
        )
        .expect("verified previews");
        assert_eq!(previews.patch.as_deref(), std::str::from_utf8(patch).ok());
        assert_eq!(
            previews.manifest.as_deref(),
            std::str::from_utf8(manifest).ok()
        );

        let mut wrong_run = patch_artifact.clone();
        wrong_run.storage_key = format!("{}/patch-{}.patch", Uuid::new_v4(), wrong_run.sha256);
        assert!(matches!(
            read_preview(temporary.path(), run_id, &wrong_run, "patch"),
            Err(RunError::PreviewUnavailable)
        ));

        let oversized = PreviewArtifact {
            size_bytes: i64::try_from(MAX_RESULT_PREVIEW_BYTES + 1).expect("preview ceiling"),
            ..patch_artifact
        };
        assert_eq!(
            read_preview(temporary.path(), run_id, &oversized, "patch")
                .expect("oversized previews are omitted"),
            None
        );
    }
}
