//! Independently compare persisted metadata to the real host files and journals.

use super::support::Fixture;
use sqlx::PgPool;
use std::{fs, os::unix::fs::MetadataExt, path::Path};
use volume_trait::{OwnedBackingPhase, OwnedProvisioningContext};

#[derive(sqlx::FromRow)]
struct Persisted {
    state: String,
    provisioning_state: String,
    host_path: String,
    owner_namespace: uuid::Uuid,
    canonical_bytes: Vec<u8>,
    proof_json: serde_json::Value,
}

pub async fn verify(fixture: &Fixture, ready: &OwnedProvisioningContext) {
    let purpose = &fixture.request.purpose;
    let registration = purpose.receipt().intent.registration();
    let observation = ready
        .observation
        .as_ref()
        .expect("actual physical Ready observation");
    let fields = observation.fields();
    assert_eq!(fields.phase, OwnedBackingPhase::Ready);
    let row:Persisted=sqlx::query_as("SELECT volume.state,volume.provisioning_state,volume.host_path,purpose.owner_namespace,purpose.canonical_bytes,observation.proof_json FROM agent_instance_state_volumes volume JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id JOIN volume_owned_backing_observations observation ON observation.volume_id=volume.id WHERE volume.id=$1 AND observation.id=$2")
        .bind(registration.id().as_uuid()).bind(fields.id).fetch_one(&fixture.sql.pool).await.expect("real SQL birth/state evidence");
    assert_eq!(row.state, "ready");
    assert_eq!(row.provisioning_state, "ready");
    assert_eq!(
        row.host_path,
        purpose.canonical_path().to_str().expect("fixture path")
    );
    assert_eq!(row.owner_namespace, fixture.owner.namespace_id().as_uuid());
    assert_eq!(row.canonical_bytes, purpose.canonical_bytes());
    assert_eq!(
        row.proof_json,
        serde_json::to_value(fields).expect("exact SQL/DTO wire")
    );
    verify_correlation(fixture, ready).await;
    let namespace = purpose.root().join(purpose.namespace_name());
    let root = fs::metadata(purpose.root()).expect("actual root");
    let directory = fs::metadata(&namespace).expect("actual namespace");
    let backing = fs::metadata(purpose.canonical_path()).expect("actual singly owned backing");
    assert_eq!(
        (fields.root_device, fields.root_inode),
        (root.dev(), root.ino())
    );
    assert_eq!(
        (fields.namespace_device, fields.namespace_inode),
        (directory.dev(), directory.ino())
    );
    assert_eq!(
        (fields.backing_device, fields.backing_inode),
        (backing.dev(), backing.ino())
    );
    assert_eq!(fields.owner_uid, backing.uid());
    assert_eq!(backing.uid(), directory.uid());
    assert_eq!(backing.nlink(), 1);
    assert_eq!(backing.mode() & 0o077, 0);
    assert_eq!(backing.len(), registration.capacity_bytes());
    verify_journal(fixture, ready, &namespace).await;
    filesystem(fixture, ready);
    assert_eq!(fixture.format_count(), 1);
    let fsck =
        fs::read_to_string(fixture.tools.path().join("fsck-args")).expect("real readonly fsck ran");
    assert!(fsck.lines().all(|args| args.starts_with("-f -n /proc/")));
}

async fn verify_journal(fixture: &Fixture, ready: &OwnedProvisioningContext, namespace: &Path) {
    let purpose = &fixture.request.purpose;
    let fields = ready
        .observation
        .as_ref()
        .expect("actual Ready evidence")
        .fields();
    let birth = fs::read(namespace.join("birth")).expect("complete birth record");
    let inode = fs::read(namespace.join("inode-00")).expect("exact first recorded inode");
    let birth_hash = digest(&fixture.sql.pool, &birth).await;
    let inode_hash = digest(&fixture.sql.pool, &inode).await;
    assert_eq!(fields.birth_record_hash, birth_hash);
    assert_eq!(fields.inode_record_hash, inode_hash);
    // Birth header is fixed13 bytes; payload embeds the exact checked purpose.
    assert_eq!(
        &birth[13..13 + purpose.canonical_bytes().len()],
        purpose.canonical_bytes()
    );
    let mut evidence = birth_hash.to_vec();
    evidence.extend_from_slice(&inode_hash);
    for name in [
        "allocation-intent",
        "allocated",
        "published",
        "format-intent",
    ] {
        evidence.push(1);
        evidence.extend_from_slice(
            &fs::read(namespace.join(name)).expect("complete actual phase record"),
        );
    }
    assert_eq!(
        fields.journal_hash,
        digest(&fixture.sql.pool, &evidence).await
    );
}

async fn verify_correlation(fixture: &Fixture, ready: &OwnedProvisioningContext) {
    let receipt = fixture.request.purpose.receipt();
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM volume_creation_seals seal JOIN volume_owned_provisioning_operations operation ON operation.volume_id=seal.volume_id JOIN application_events event ON event.id=operation.event_id WHERE seal.id=$1 AND seal.canonical_bytes=$2 AND operation.id=$3 AND operation.actor_id=$4 AND operation.request_id=$5 AND operation.event_id=$6 AND operation.generation=$7 AND event.scope_kind='project' AND event.scope_id=seal.project_id AND event.event_type='project.changed')")
        .bind(receipt.seal_id.as_uuid()).bind(receipt.intent.canonical_bytes()).bind(ready.claim.operation_id).bind(fixture.identity.user_id.as_uuid()).bind(ready.claim.request_id).bind(ready.claim.event_id).bind(i64::try_from(ready.claim.generation).expect("bounded generation"))
        .fetch_one(&fixture.sql.pool).await.expect("actual original seal and operation correlation");
    assert!(
        exact,
        "fixture ownership boundary preserves full original command/effect/attempt values; no engine claim is asserted"
    );
}

fn filesystem(fixture: &Fixture, ready: &OwnedProvisioningContext) {
    let bytes = fs::read(fixture.request.purpose.canonical_path()).expect("actual native ext4");
    let block = &bytes[1024..2048];
    let birth = ready
        .observation
        .as_ref()
        .expect("Ready")
        .fields()
        .filesystem
        .as_ref()
        .expect("readonly initial filesystem birth");
    assert_eq!(&block[56..58], &[0x53, 0xef]);
    assert_eq!(&block[104..120], birth.filesystem_uuid.as_bytes());
    assert_eq!(
        u32::from_le_bytes(block[24..28].try_into().expect("log block size")),
        2
    );
    assert_eq!(
        u64::from(u32::from_le_bytes(
            block[4..8].try_into().expect("block count")
        )),
        birth.block_count
    );
    assert_eq!(
        birth.block_count * 4096,
        fixture
            .request
            .purpose
            .receipt()
            .intent
            .registration()
            .capacity_bytes()
    );
    assert!(birth.clean);
    assert_eq!(birth.block_size, 4096);
}

pub async fn digest(pool: &PgPool, bytes: &[u8]) -> [u8; 32] {
    let hash: Vec<u8> = sqlx::query_scalar("SELECT sha256($1::bytea)")
        .bind(bytes)
        .fetch_one(pool)
        .await
        .expect("independent SHA256 of actual host bytes");
    hash.try_into().expect("SHA256 length")
}

/// Byte hashes AND inode identities for all actual regular files beneath root.
pub async fn tree(pool: &PgPool, root: &Path) -> Vec<(String, u64, u64, u32, [u8; 32])> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("task root entries") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let stat = entry.metadata().expect("entry metadata");
            if stat.is_dir() {
                pending.push(path);
            } else {
                assert!(
                    stat.is_file(),
                    "fixture contains only directories and regular files"
                );
                files.push((
                    path.strip_prefix(root)
                        .expect("relative path")
                        .to_string_lossy()
                        .into_owned(),
                    stat.dev(),
                    stat.ino(),
                    stat.uid(),
                    digest(pool, &fs::read(path).expect("actual host bytes")).await,
                ));
            }
        }
    }
    files.sort();
    files
}
