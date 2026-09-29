// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
pub(crate) fn canonical_source(source_root: &Path, name: &str) -> PathBuf {
    source_root.join(name)
}

pub(crate) fn copy_source_tree(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "canonical cooking source is not a directory: {}",
                source.display()
            ),
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == "__pycache__" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_file()
            && matches!(
                source_path.extension().and_then(std::ffi::OsStr::to_str),
                Some("pyc" | "pyo")
            )
        {
            continue;
        }
        if metadata.is_dir() {
            copy_source_tree(&source_path, &destination_path)?;
        } else if metadata.file_type().is_symlink() {
            unix_fs::symlink(fs::read_link(source_path)?, destination_path)?;
        } else if metadata.is_file() {
            fs::copy(source_path, destination_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical cooking source contains unsupported file type",
            ));
        }
    }
    Ok(())
}

pub(crate) async fn initialize_git(source: &Path, display_name: &str) -> Result<(), BuildError> {
    git(source, &["init", "--initial-branch=main"]).await?;
    git(
        source,
        &["config", "user.email", "cooking-fixture@example.invalid"],
    )
    .await?;
    git(source, &["config", "user.name", display_name]).await?;
    git(source, &["add", "--all"]).await?;
    git(source, &["commit", "--message", "canonical cooking source"]).await?;
    Ok(())
}

pub(crate) async fn git(directory: &Path, arguments: &[&str]) -> Result<(), BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid_state(&format!(
            "Git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

pub(crate) async fn git_output(directory: &Path, arguments: &[&str]) -> Result<String, BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Err(invalid_state(
            "Git did not produce the cooking source commit",
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub(crate) async fn authenticated_git(
    directory: &Path,
    token: &str,
    arguments: &[&str],
) -> Result<(), BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid_state(&format!(
            "authenticated Git push failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

pub(crate) fn invalid_state(message: &str) -> BuildError {
    io::Error::other(message.to_owned()).into()
}

pub(crate) fn redact_build_log(line: &str) -> String {
    line.replace("golden-brokered-provider-sentinel-5d1a", "[REDACTED]")
        .replace("cooking-inbound-only-fixture-sentinel", "[REDACTED]")
        .replace("cooking-model-only-fixture-sentinel-724c", "[REDACTED]")
        .replace("cooking-relay-only-fixture-sentinel-819e", "[REDACTED]")
        .replace("cooking-model-rotated-fixture-sentinel-936f", "[REDACTED]")
        .replace(
            "cooking-inbound-rotated-fixture-sentinel-157a",
            "[REDACTED]",
        )
        .replace("cooking-relay-rotated-fixture-sentinel-482b", "[REDACTED]")
        .replace("telegram-bot-api-secret-token", "[REDACTED]")
}
