//! Git credential-helper protocol adapter for developer PATs.

use pat_domain::PersonalAccessToken;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::ExitCode,
};
use zeroize::Zeroizing;

const MAX_INPUT_BYTES: u64 = 16 * 1024;
const MAX_AUTHORITY_BYTES: usize = 255;

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("git-credential-hephaestus: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut arguments: impl Iterator<Item = String>) -> Result<(), HelperError> {
    let operation = arguments.next().ok_or(HelperError::Usage)?;
    let root = credential_root()?;
    match operation.as_str() {
        "get" => {
            reject_extra_arguments(arguments)?;
            let request = read_git_request(io::stdin().lock())?;
            let Some(authority) = request.https_authority()? else {
                return Ok(());
            };
            if let Some(token) = load(&root, authority)? {
                let mut stdout = io::stdout().lock();
                stdout.write_all(b"username=pat\npassword=")?;
                stdout.write_all(token.as_bytes())?;
                stdout.write_all(b"\n\n")?;
                stdout.flush()?;
            }
            Ok(())
        }
        "store" => {
            reject_extra_arguments(arguments)?;
            let request = read_git_request(io::stdin().lock())?;
            let authority = request
                .https_authority()?
                .ok_or(HelperError::HttpsRequired)?;
            let password = request
                .fields
                .get("password")
                .ok_or(HelperError::MissingToken)?;
            store(&root, authority, password)
        }
        "erase" => {
            reject_extra_arguments(arguments)?;
            let request = read_git_request(io::stdin().lock())?;
            if let Some(authority) = request.https_authority()? {
                erase(&root, authority)?;
            }
            Ok(())
        }
        "login" => {
            let authority = arguments.next().ok_or(HelperError::Usage)?;
            reject_extra_arguments(arguments)?;
            validate_authority(&authority)?;
            let mut token = Zeroizing::new(String::new());
            io::stdin()
                .take(MAX_INPUT_BYTES)
                .read_to_string(&mut token)?;
            let trimmed = token.trim_end_matches(['\r', '\n']);
            store(&root, &authority, trimmed)
        }
        _ => Err(HelperError::Usage),
    }
}

fn reject_extra_arguments(mut arguments: impl Iterator<Item = String>) -> Result<(), HelperError> {
    if arguments.next().is_some() {
        Err(HelperError::Usage)
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct GitRequest {
    fields: BTreeMap<String, String>,
}

impl GitRequest {
    fn https_authority(&self) -> Result<Option<&str>, HelperError> {
        if self.fields.get("protocol").map(String::as_str) != Some("https") {
            return Ok(None);
        }
        let authority = self
            .fields
            .get("host")
            .ok_or(HelperError::MissingAuthority)?;
        validate_authority(authority)?;
        Ok(Some(authority))
    }
}

fn read_git_request(mut input: impl Read) -> Result<GitRequest, HelperError> {
    let mut text = String::new();
    input
        .by_ref()
        .take(MAX_INPUT_BYTES)
        .read_to_string(&mut text)?;
    if text.len() >= usize::try_from(MAX_INPUT_BYTES).expect("input limit fits usize") {
        return Err(HelperError::OversizedInput);
    }
    let mut fields = BTreeMap::new();
    for line in text.lines().take_while(|line| !line.is_empty()) {
        let (key, value) = line.split_once('=').ok_or(HelperError::MalformedInput)?;
        if key.is_empty()
            || key.chars().any(char::is_control)
            || value
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0'))
            || fields.insert(key.to_owned(), value.to_owned()).is_some()
        {
            return Err(HelperError::MalformedInput);
        }
    }
    Ok(GitRequest { fields })
}

fn credential_root() -> Result<PathBuf, HelperError> {
    if let Some(value) = env::var_os("HEPHAESTUS_GIT_CREDENTIAL_ROOT") {
        return absolute(PathBuf::from(value));
    }
    if let Some(value) = env::var_os("XDG_DATA_HOME") {
        return Ok(absolute(PathBuf::from(value))?.join("hephaestus/git-credentials"));
    }
    let home = env::var_os("HOME").ok_or(HelperError::MissingStorageRoot)?;
    Ok(absolute(PathBuf::from(home))?.join(".local/share/hephaestus/git-credentials"))
}

fn absolute(path: PathBuf) -> Result<PathBuf, HelperError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(HelperError::StorageRootMustBeAbsolute)
    }
}

fn validate_authority(authority: &str) -> Result<(), HelperError> {
    if authority.is_empty()
        || authority.len() > MAX_AUTHORITY_BYTES
        || authority.trim() != authority
        || authority.contains(['/', '\\', '@'])
        || authority.chars().any(char::is_control)
    {
        Err(HelperError::InvalidAuthority)
    } else {
        Ok(())
    }
}

fn credential_path(root: &Path, authority: &str) -> PathBuf {
    let digest = Sha256::digest(authority.as_bytes());
    let mut name = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut name, "{byte:02x}").expect("writing to String cannot fail");
    }
    root.join(name)
}

fn ensure_root(root: &Path) -> Result<(), HelperError> {
    fs::create_dir_all(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.file_type().is_dir() || metadata.mode() & 0o077 != 0 {
        return Err(HelperError::UnsafeStorage);
    }
    Ok(())
}

fn store(root: &Path, authority: &str, token: &str) -> Result<(), HelperError> {
    validate_authority(authority)?;
    PersonalAccessToken::parse(token).map_err(|_| HelperError::InvalidToken)?;
    ensure_root(root)?;
    let destination = credential_path(root, authority);
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| -> Result<(), HelperError> {
        file.write_all(authority.as_bytes())?;
        file.write_all(b"\n")?;
        file.write_all(token.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ignored = fs::remove_file(&temporary);
    }
    result
}

fn load(root: &Path, authority: &str) -> Result<Option<Zeroizing<String>>, HelperError> {
    let path = credential_path(root, authority);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != fs::metadata(root)?.uid()
    {
        return Err(HelperError::UnsafeStorage);
    }
    let mut contents = Zeroizing::new(String::new());
    fs::File::open(path)?
        .take(MAX_INPUT_BYTES)
        .read_to_string(&mut contents)?;
    let (stored_authority, token) = contents
        .trim_end_matches(['\r', '\n'])
        .split_once('\n')
        .ok_or(HelperError::UnsafeStorage)?;
    if stored_authority != authority {
        return Err(HelperError::UnsafeStorage);
    }
    PersonalAccessToken::parse(token).map_err(|_| HelperError::UnsafeStorage)?;
    Ok(Some(Zeroizing::new(token.to_owned())))
}

fn erase(root: &Path, authority: &str) -> Result<(), HelperError> {
    let path = credential_path(root, authority);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, thiserror::Error)]
enum HelperError {
    #[error(
        "usage: git-credential-hephaestus <get|store|erase> or git-credential-hephaestus login AUTHORITY"
    )]
    Usage,
    #[error("credential helper input is malformed")]
    MalformedInput,
    #[error("credential helper input is too large")]
    OversizedInput,
    #[error("only HTTPS credentials are stored")]
    HttpsRequired,
    #[error("the Git authority is missing")]
    MissingAuthority,
    #[error("the Git authority is invalid")]
    InvalidAuthority,
    #[error("the personal access token is missing")]
    MissingToken,
    #[error("the personal access token is invalid")]
    InvalidToken,
    #[error("no credential storage root is available")]
    MissingStorageRoot,
    #[error("the credential storage root must be absolute")]
    StorageRootMustBeAbsolute,
    #[error("credential storage permissions or contents are unsafe")]
    UnsafeStorage,
    #[error("local credential storage failed")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::{credential_path, load, read_git_request, store};
    use pat_domain::{PersonalAccessToken, PersonalAccessTokenId};
    use std::{fs, io::Cursor, os::unix::fs::PermissionsExt as _};

    fn token() -> String {
        PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), [7; 32]).expose()
    }

    #[test]
    fn git_protocol_is_strict_and_authority_bound() {
        let request = read_git_request(Cursor::new(b"protocol=https\nhost=git.example\n\n"))
            .expect("valid request");
        assert_eq!(
            request.https_authority().expect("valid authority"),
            Some("git.example")
        );
        assert!(read_git_request(Cursor::new(b"protocol=https\nhost=a\nhost=b\n")).is_err());
        assert!(read_git_request(Cursor::new(b"protocol=https\nhost=bad/path\n")).is_ok());
        assert!(
            read_git_request(Cursor::new(b"protocol=https\nhost=bad/path\n"))
                .expect("parse fields")
                .https_authority()
                .is_err()
        );
    }

    #[test]
    fn storage_is_private_and_exact_authority_scoped() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("credentials");
        let value = token();
        store(&root, "git.example", &value).expect("store token");

        let stored = load(&root, "git.example")
            .expect("load token")
            .expect("stored token");
        assert_eq!(stored.as_str(), value);
        assert!(
            load(&root, "other.example")
                .expect("other lookup")
                .is_none()
        );
        assert_eq!(
            fs::metadata(credential_path(&root, "git.example"))
                .expect("credential metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
