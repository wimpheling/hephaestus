use crate::{
    backing, invalid_backing,
    owned_journal::filesystem::{self, FileIdentity},
};
use rustix::fs::{Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Write, os::unix::fs::MetadataExt, path::Path};
use uuid::Uuid;
use volume_trait::{VolumeError, VolumeRootNamespaceId};

pub const NAME: &str = ".heph-volume-root.owner";
pub const PENDING: &str = ".heph-volume-root.owner.pending";
const MAGIC: &[u8] = b"HEPHVROOT1";

pub fn open_root(path: &Path) -> Result<File, VolumeError> {
    if !path.is_absolute() || std::fs::canonicalize(path).map_err(backing)? != path {
        return Err(invalid_backing("owned volume root must be canonical"));
    }
    let file = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(backing)?
    .into();
    filesystem::validate_directory(&file, false).map_err(backing)?;
    Ok(file)
}
pub fn read(root: &File) -> Result<(File, Vec<u8>), VolumeError> {
    let file = filesystem::open_file(root, NAME, false).map_err(backing)?;
    filesystem::validate_private_file(&file).map_err(backing)?;
    let bytes = filesystem::read_record(root, NAME)
        .map_err(backing)?
        .ok_or_else(|| invalid_backing("root marker disappeared"))?;
    Ok((file, bytes))
}
pub fn load_or_create(
    root: &File,
    path: &Path,
    host: &str,
    identity: FileIdentity,
) -> Result<(File, Vec<u8>, VolumeRootNamespaceId), VolumeError> {
    if filesystem::exists(root, PENDING).map_err(backing)? {
        return Err(invalid_backing(
            "incomplete root owner publication requires recovery",
        ));
    }
    if filesystem::exists(root, NAME).map_err(backing)? {
        let (file, bytes) = read(root)?;
        let namespace = decode(&bytes, identity, host)?;
        return Ok((file, bytes, namespace));
    }
    for (count, entry) in std::fs::read_dir(path).map_err(backing)?.enumerate() {
        let name = entry.map_err(backing)?.file_name();
        if count >= 4096 || name.as_encoded_bytes().starts_with(b".owned-") {
            return Err(invalid_backing(
                "missing root owner with owned or unbounded namespace evidence",
            ));
        }
    }
    if FileIdentity::of(&open_root(path)?).map_err(backing)? != identity {
        return Err(invalid_backing("root changed before owner initialization"));
    }
    let namespace = VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).map_err(backing)?;
    let bytes = encode(
        namespace,
        identity,
        root.metadata().map_err(backing)?.uid(),
        host,
    )?;
    // EXCL pending record serializes initialization; interrupted writes remain held.
    let mut pending = filesystem::create_file(root, PENDING).map_err(backing)?;
    pending.write_all(&bytes).map_err(backing)?;
    pending.sync_all().map_err(backing)?;
    filesystem::rename_new(root, PENDING, root, NAME).map_err(backing)?;
    root.sync_all().map_err(backing)?;
    let (file, actual) = read(root)?;
    if actual != bytes {
        return Err(VolumeError::IntentConflict);
    }
    Ok((file, actual, namespace))
}
fn encode(
    namespace: VolumeRootNamespaceId,
    identity: FileIdentity,
    uid: u32,
    host: &str,
) -> Result<Vec<u8>, VolumeError> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(namespace.as_uuid().as_bytes());
    identity.encode(&mut bytes);
    bytes.extend_from_slice(&uid.to_be_bytes());
    bytes.extend_from_slice(&u16::try_from(host.len()).map_err(backing)?.to_be_bytes());
    bytes.extend_from_slice(host.as_bytes());
    let hash = Sha256::digest(&bytes);
    bytes.extend_from_slice(&hash);
    Ok(bytes)
}
pub fn decode(
    bytes: &[u8],
    identity: FileIdentity,
    host: &str,
) -> Result<VolumeRootNamespaceId, VolumeError> {
    let header = MAGIC.len() + 16 + 16 + 4 + 2;
    if bytes.len() != header + host.len() + 32 || bytes.get(..MAGIC.len()) != Some(MAGIC) {
        return Err(invalid_backing("root owner marker is truncated or unknown"));
    }
    let offset = MAGIC.len() + 16;
    let mut expected = Vec::new();
    identity.encode(&mut expected);
    expected.extend_from_slice(&rustix::process::geteuid().as_raw().to_be_bytes());
    expected.extend_from_slice(&u16::try_from(host.len()).map_err(backing)?.to_be_bytes());
    expected.extend_from_slice(host.as_bytes());
    if bytes[offset..bytes.len() - 32] != expected
        || bytes[bytes.len() - 32..] != Sha256::digest(&bytes[..bytes.len() - 32])[..]
    {
        return Err(invalid_backing(
            "root owner marker does not bind this physical root and host",
        ));
    }
    VolumeRootNamespaceId::from_uuid(
        Uuid::from_slice(&bytes[MAGIC.len()..offset]).map_err(backing)?,
    )
    .map_err(backing)
}
