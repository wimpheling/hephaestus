//! Exact canonical birth, inode and phase records.

use super::{
    JournalError,
    codec::{self, Cursor, Kind},
    filesystem::FileIdentity,
};
use uuid::Uuid;

pub struct Birth {
    pub root: FileIdentity,
    pub namespace: FileIdentity,
    pub hash: [u8; 32],
}
impl Birth {
    pub fn encode(
        purpose: &[u8],
        root: FileIdentity,
        namespace: FileIdentity,
    ) -> Result<Vec<u8>, JournalError> {
        let mut payload = purpose.to_vec();
        root.encode(&mut payload);
        namespace.encode(&mut payload);
        codec::encode(Kind::Birth, &payload)
    }
    pub fn decode(bytes: &[u8], purpose: &[u8]) -> Result<Self, JournalError> {
        let mut cursor = Cursor::new(codec::decode(Kind::Birth, bytes)?);
        if cursor.take(purpose.len())? != purpose {
            return Err(JournalError::Conflict("birth purpose differs"));
        }
        let root = identity(&mut cursor)?;
        let namespace = identity(&mut cursor)?;
        cursor.finish()?;
        Ok(Self {
            root,
            namespace,
            hash: codec::digest(bytes),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InodeRecord {
    pub slot: u8,
    pub stage: Uuid,
    pub inode: FileIdentity,
    pub birth_hash: [u8; 32],
    pub hash: [u8; 32],
}
impl InodeRecord {
    pub fn new(
        slot: u8,
        stage: Uuid,
        inode: FileIdentity,
        birth_hash: [u8; 32],
    ) -> Result<Self, JournalError> {
        let mut value = Self {
            slot,
            stage,
            inode,
            birth_hash,
            hash: [0; 32],
        };
        value.hash = codec::digest(&value.encode()?);
        Ok(value)
    }
    pub fn name(&self) -> String {
        format!("stage-{}.raw", self.stage)
    }
    pub fn encode(&self) -> Result<Vec<u8>, JournalError> {
        let mut payload = self.birth_hash.to_vec();
        payload.push(self.slot);
        payload.extend_from_slice(self.stage.as_bytes());
        self.inode.encode(&mut payload);
        codec::encode(Kind::Inode, &payload)
    }
    pub fn decode(bytes: &[u8], expected_birth: &[u8; 32]) -> Result<Self, JournalError> {
        let mut cursor = Cursor::new(codec::decode(Kind::Inode, bytes)?);
        let birth_hash = cursor.array()?;
        let slot = cursor.array::<1>()?[0];
        let stage = Uuid::from_bytes(cursor.array()?);
        let inode = identity(&mut cursor)?;
        cursor.finish()?;
        if &birth_hash != expected_birth || slot >= super::MAX_ATTEMPTS || stage.is_nil() {
            return Err(JournalError::RecoveryRequired(
                "inode record belongs to another birth or attempt",
            ));
        }
        Ok(Self {
            slot,
            stage,
            inode,
            birth_hash,
            hash: codec::digest(bytes),
        })
    }
}
pub fn attempt(slot: u8, stage: Uuid, birth: &[u8; 32]) -> Result<Vec<u8>, JournalError> {
    let mut payload = birth.to_vec();
    payload.push(slot);
    payload.extend_from_slice(stage.as_bytes());
    codec::encode(Kind::Attempt, &payload)
}
pub fn check_attempt(bytes: &[u8], inode: &InodeRecord) -> Result<(), JournalError> {
    if bytes != attempt(inode.slot, inode.stage, &inode.birth_hash)? {
        return Err(JournalError::RecoveryRequired(
            "inode record has no exact durable staging attempt",
        ));
    }
    Ok(())
}
pub fn phase(kind: Kind, inode: &InodeRecord) -> Result<Vec<u8>, JournalError> {
    let mut payload = inode.birth_hash.to_vec();
    payload.extend_from_slice(&inode.hash);
    codec::encode(kind, &payload)
}
pub fn check_phase(kind: Kind, bytes: &[u8], inode: &InodeRecord) -> Result<(), JournalError> {
    codec::decode(kind, bytes)?;
    if bytes != phase(kind, inode)? {
        return Err(JournalError::RecoveryRequired(
            "phase record belongs to another inode",
        ));
    }
    Ok(())
}
fn identity(cursor: &mut Cursor<'_>) -> Result<FileIdentity, JournalError> {
    let device = u64::from_be_bytes(cursor.array()?);
    let inode = u64::from_be_bytes(cursor.array()?);
    if inode == 0 {
        return Err(JournalError::RecoveryRequired(
            "journal inode identity is empty",
        ));
    }
    Ok(FileIdentity { device, inode })
}
