//! Authorized, bounded inspection of canonical bare Git repositories.

use sqlx::PgPool;
use std::sync::Arc;

const MAX_CHANGED_FILES: usize = 100;
const MAX_HUNKS_PER_FILE: usize = 50;
const MAX_LINES_PER_HUNK: usize = 500;
const MAX_DIFF_BYTES_PER_FILE: usize = 131_072;

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("repository access is denied")]
    PermissionDenied,
    #[error("repository object was not found")]
    NotFound,
    #[error("repository browser input is invalid")]
    InvalidArgument,
    #[error("repository browser limit exceeded")]
    ResourceExhausted,
    #[error("repository query failed")]
    Persistence(#[source] sqlx::Error),
    #[error("repository storage failed")]
    Storage(#[source] forge_service::GitStorageError),
    #[error("Git inspection failed")]
    Git,
}

#[derive(Clone)]
pub struct Branch {
    pub name: String,
    pub git_ref: String,
    pub commit: String,
    pub committed_at: i64,
    pub subject: String,
}

pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub subject: String,
}

pub struct CommitDetail {
    pub commit: Commit,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: i64,
    pub body: String,
    pub selected_parent: String,
    pub files: Vec<DiffFile>,
    pub truncated: bool,
}

pub struct CommitDetailRequest<'a> {
    pub branch: &'a str,
    pub commit: &'a str,
    pub parent: &'a str,
    pub skip: usize,
    pub limit: usize,
}

pub struct DiffFile {
    pub path: String,
    pub previous_path: String,
    pub state: DiffFileState,
    pub additions: u64,
    pub deletions: u64,
    pub hunks: Vec<DiffHunk>,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
pub enum DiffFileState {
    Added,
    Deleted,
    Modified,
    Renamed,
    Binary,
    Truncated,
    Unavailable,
}

pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
}

pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub object_id: String,
    pub size: Option<u64>,
    pub path: String,
}

pub struct BrowserApplication {
    pool: PgPool,
    storage: Arc<forge_service::GitStorage>,
}

mod commit;
mod diff_ops;
mod diff_parse;
mod git;
mod methods;

#[cfg(test)]
mod tests {
    use super::{DiffFileState, DiffLineKind, MAX_LINES_PER_HUNK};
    use super::{
        commit::select_parent,
        diff_ops::{changed_files, populate_hunks},
        diff_parse::parse_hunks,
        git::{parse_commits, parse_tree, validate_path},
    };
    use std::{fs, path::Path, process::Command};
    use tempfile::TempDir;

    #[test]
    fn rejects_traversal_and_malformed_git_records() {
        assert!(validate_path("src/lib.rs").is_ok());
        assert!(validate_path("../secret").is_err());
        assert!(parse_commits(b"incomplete\0").is_err());
        assert!(parse_tree(b"100644 blob abc 1 missing-tab\0").is_err());
    }

    #[test]
    fn parses_root_commit_with_empty_parent_field() {
        let commits =
            parse_commits(b"abc\0\0Root Author\0root@example.test\x001700000000\0initial\0")
                .expect("root commit should parse");
        assert_eq!(commits.len(), 1);
        assert!(commits[0].parents.is_empty());
        assert_eq!(commits[0].subject, "initial");
    }

    #[test]
    fn parses_numbered_unified_hunks_without_interpreting_source_as_markup() {
        let (hunks, additions, deletions, truncated) = parse_hunks(
            "@@ -2,2 +2,3 @@ fn sample() {\n unchanged\n-old <script>\n+new <script>\n+extra\n",
        )
        .expect("valid unified diff");
        assert!(!truncated);
        assert_eq!((additions, deletions), (2, 1));
        assert!(matches!(hunks[0].lines[1].kind, DiffLineKind::Removed));
        assert_eq!(hunks[0].lines[2].text, "new <script>");
        assert_eq!(hunks[0].lines[2].new_line, Some(3));
    }

    #[test]
    fn bounds_one_large_hunk_without_losing_its_safe_prefix() {
        let source = format!(
            "@@ -1,0 +1,{} @@\n{}",
            MAX_LINES_PER_HUNK + 1,
            "+safe\n".repeat(MAX_LINES_PER_HUNK + 1)
        );
        let (hunks, additions, _deletions, truncated) =
            parse_hunks(&source).expect("valid bounded unified diff");
        assert!(truncated);
        assert!(hunks[0].truncated);
        assert_eq!(hunks[0].lines.len(), MAX_LINES_PER_HUNK);
        assert_eq!(
            additions,
            u64::try_from(MAX_LINES_PER_HUNK).expect("fits u64")
        );
    }

    #[test]
    fn root_and_merge_parent_selection_are_deterministic() {
        assert_eq!(select_parent(&[], "").expect("root parent"), "");
        assert_eq!(
            select_parent(&["a".repeat(40), "b".repeat(40)], "").expect("first parent"),
            "a".repeat(40)
        );
        assert!(select_parent(&["a".repeat(40)], &"b".repeat(40)).is_err());
    }

    #[tokio::test]
    async fn real_git_diff_marks_rename_and_binary_without_exposing_binary_bytes() {
        let fixture = repository_fixture();
        let repository = fixture.path().join(".git");
        let root = git_at(fixture.path(), ["rev-parse", "HEAD"]);

        fs::rename(
            fixture.path().join("recipe.txt"),
            fixture.path().join("renamed-recipe.txt"),
        )
        .expect("rename fixture file");
        fs::write(fixture.path().join("renamed-recipe.txt"), "soup\nstew\n")
            .expect("modify renamed fixture");
        fs::write(fixture.path().join("image.bin"), [0, 159, 146, 150]).expect("binary fixture");
        git_at(fixture.path(), ["add", "-A"]);
        git_at(
            fixture.path(),
            ["commit", "--quiet", "-m", "rename and binary"],
        );
        let commit = git_at(fixture.path(), ["rev-parse", "HEAD"]);

        let (mut files, truncated) = changed_files(&repository, &commit, &root)
            .await
            .expect("bounded file list");
        assert!(!truncated);
        for file in &mut files {
            populate_hunks(&repository, &commit, &root, file)
                .await
                .expect("safe per-file inspection");
        }

        let renamed = files
            .iter()
            .find(|file| file.path == "renamed-recipe.txt")
            .expect("renamed file summary");
        assert!(matches!(renamed.state, DiffFileState::Renamed));
        assert_eq!(renamed.previous_path, "recipe.txt");
        let binary = files
            .iter()
            .find(|file| file.path == "image.bin")
            .expect("binary file summary");
        assert!(matches!(binary.state, DiffFileState::Binary));
        assert!(binary.hunks.is_empty());

        fs::remove_file(fixture.path().join("renamed-recipe.txt")).expect("delete fixture");
        git_at(fixture.path(), ["add", "-A"]);
        git_at(fixture.path(), ["commit", "--quiet", "-m", "delete recipe"]);
        let deletion = git_at(fixture.path(), ["rev-parse", "HEAD"]);
        let (files, truncated) = changed_files(&repository, &deletion, &commit)
            .await
            .expect("bounded deletion list");
        assert!(!truncated);
        assert!(files.iter().any(|file| {
            file.path == "renamed-recipe.txt" && matches!(file.state, DiffFileState::Deleted)
        }));
    }

    fn repository_fixture() -> TempDir {
        let fixture = tempfile::tempdir().expect("temporary Git repository");
        git_at(fixture.path(), ["init", "--quiet"]);
        git_at(fixture.path(), ["config", "user.name", "Hephaestus test"]);
        git_at(
            fixture.path(),
            ["config", "user.email", "test@hephaestus.invalid"],
        );
        fs::write(fixture.path().join("recipe.txt"), "soup\n").expect("root fixture");
        git_at(fixture.path(), ["add", "recipe.txt"]);
        git_at(fixture.path(), ["commit", "--quiet", "-m", "root recipe"]);
        fixture
    }

    fn git_at<const N: usize>(directory: &Path, arguments: [&str; N]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .expect("run Git fixture command");
        assert!(
            output.status.success(),
            "Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("Git fixture output")
            .trim()
            .to_owned()
    }
}
