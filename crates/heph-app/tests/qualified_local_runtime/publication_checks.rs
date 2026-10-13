//! Read-only pins and exact fresh-public107 producer inventory for owned drain.

use super::super::build_helpers::BuildError;
use super::{PublishedVariant, publication_rows};
use release_domain::ReleaseCommandKey;
use rustix::fs::{Mode, OFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    os::unix::fs::{FileExt, MetadataExt},
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;
use vm_trait::VmId;

type Command = (
    Vec<u8>,
    String,
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<Vec<u8>>,
);

struct FilePin {
    relative: PathBuf,
    file: File,
    identity: (u64, u64, u64, u32),
    hash: [u8; 32],
}

/// Exact source/artifact descriptors and immutable database projection.
pub(in super::super) struct PublicationPins {
    root: PathBuf,
    directory: File,
    files: Vec<FilePin>,
    rows: Value,
}

/// A controlled producer set, with immutable public107 migration history.
pub(in super::super) struct PublicationCheckpoint {
    pub(in super::super) expected: [VmId; 2],
    state: Value,
    isolated: File,
    active: File,
}

impl PublicationPins {
    pub(in super::super) async fn capture(
        p: &PublishedVariant,
        root: &Path,
    ) -> Result<Self, BuildError> {
        if !root.is_absolute() || fs::canonicalize(root)? != root {
            return Err(held("noncanonical publication pin root"));
        }
        let directory = open_root(root)?;
        let rows = publication_rows(p).await?;
        let key: Uuid = serde_json::from_value(rows["artifact"][3].clone())?;
        let mut pins = Self {
            root: root.into(),
            directory,
            files: Vec::new(),
            rows,
        };
        for parent in [
            p.source(),
            p.published.source_path.as_path(),
            p.published.working_path.as_path(),
        ] {
            for name in ["agent.toml", "app.py", "build.sh"] {
                pins.pin(&parent.join(name))?;
            }
        }
        pins.pin(
            &p.runtime
                .release_artifact_root
                .join(key.simple().to_string()),
        )?;
        for index in 0..3 {
            if pins.files[index].hash != pins.files[index + 3].hash
                || pins.files[index].hash != pins.files[index + 6].hash
            {
                return Err(held(
                    "published source bytes differ from the genuine original",
                ));
            }
        }
        if pins.files[9].hash != pins.files[1].hash
            || pins.rows["artifact"][4] != json!(pins.files[9].hash.to_vec())
            || pins.rows["artifact"][5] != json!(pins.files[9].identity.2)
        {
            return Err(held("actual imported artifact bytes differ"));
        }
        pins.verify()?;
        Ok(pins)
    }

    fn pin(&mut self, path: &Path) -> Result<(), BuildError> {
        let relative = path.strip_prefix(&self.root)?.to_path_buf();
        let file = open_relative(&self.directory, &relative)?;
        let identity = file_identity(&file)?;
        let hash = file_hash(&file)?;
        self.files.push(FilePin {
            relative,
            file,
            identity,
            hash,
        });
        Ok(())
    }

    pub(in super::super) fn verify(&self) -> Result<(), BuildError> {
        let current = open_root(&self.root)?;
        if fs::canonicalize(&self.root)? != self.root
            || file_identity(&current)? != file_identity(&self.directory)?
        {
            return Err(held("publication pin root changed"));
        }
        for pin in &self.files {
            let current = open_relative(&self.directory, &pin.relative)?;
            if file_identity(&pin.file)? != pin.identity
                || file_identity(&current)? != pin.identity
                || file_hash(&pin.file)? != pin.hash
                || file_hash(&current)? != pin.hash
            {
                return Err(held("pinned publication inode or content changed"));
            }
        }
        Ok(())
    }
}

fn open_root(path: &Path) -> Result<File, BuildError> {
    Ok(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}

fn open_directory(parent: &File, name: &str) -> Result<File, BuildError> {
    Ok(rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}

fn open_relative(root: &File, relative: &Path) -> Result<File, BuildError> {
    let mut directory = root.try_clone()?;
    let mut parts = relative.components().peekable();
    while let Some(component) = parts.next() {
        let Component::Normal(name) = component else {
            return Err(held("unsafe publication pin component"));
        };
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        if parts.peek().is_some() {
            flags |= OFlags::DIRECTORY;
        }
        directory = rustix::fs::openat(&directory, name, flags, Mode::empty())?.into();
    }
    if !directory.metadata()?.is_file() {
        return Err(held("publication pin is not a regular file"));
    }
    Ok(directory)
}

fn file_identity(file: &File) -> Result<(u64, u64, u64, u32), BuildError> {
    let m = file.metadata()?;
    Ok((m.dev(), m.ino(), m.len(), m.mode()))
}

fn file_hash(file: &File) -> Result<[u8; 32], BuildError> {
    let length = usize::try_from(file.metadata()?.len())?;
    let mut bytes = vec![0; length];
    file.read_exact_at(&mut bytes, 0)?;
    Ok(Sha256::digest(bytes).into())
}

pub(in super::super) async fn verify_pair_state(
    pair: &[std::sync::Arc<PublishedVariant>; 2],
    pins: &[&PublicationPins; 2],
) -> Result<PublicationCheckpoint, BuildError> {
    for (p, pin) in pair.iter().zip(pins) {
        pin.verify()?;
        if publication_rows(p).await? != pin.rows {
            return Err(held("immutable publication projection changed"));
        }
    }
    let pool = &pair[0].pool;
    let inventory: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('requests',(SELECT jsonb_agg(id ORDER BY id) FROM build_requests),'executions',(SELECT jsonb_agg(build_request_id ORDER BY build_request_id) FROM build_executions),'releases',(SELECT jsonb_agg(id ORDER BY id) FROM releases),'agents',(SELECT jsonb_agg(id ORDER BY id) FROM release_agents),'repositories',(SELECT jsonb_agg(id ORDER BY id) FROM repositories))",
    ).fetch_one(pool).await?;
    for (name, mut ids) in [
        (
            "requests",
            pair.each_ref().map(|p| p.published.build_request_id),
        ),
        (
            "executions",
            pair.each_ref().map(|p| p.published.build_request_id),
        ),
        ("releases", pair.each_ref().map(|p| p.published.release_id)),
        (
            "agents",
            pair.each_ref().map(|p| p.published.release_agent_id),
        ),
        (
            "repositories",
            pair.each_ref().map(|p| p.published.repository_id.as_uuid()),
        ),
    ] {
        ids.sort();
        if inventory[name] != json!(ids) {
            return Err(held("unexpected durable publication producer"));
        }
    }
    let controlled: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM runs) AND NOT EXISTS(SELECT 1 FROM run_requests) AND NOT EXISTS(SELECT 1 FROM command_inbox) AND NOT EXISTS(SELECT 1 FROM agent_instances) AND NOT EXISTS(SELECT 1 FROM agent_updates) AND NOT EXISTS(SELECT 1 FROM gateway_service_instances) AND NOT EXISTS(SELECT 1 FROM ui_installations) AND NOT EXISTS(SELECT 1 FROM release_ui_managed_services) AND NOT EXISTS(SELECT 1 FROM gateway_install_commands) AND NOT EXISTS(SELECT 1 FROM gateway_configure_commands) AND NOT EXISTS(SELECT 1 FROM repository_oci_image_production_jobs) AND NOT EXISTS(SELECT 1 FROM oci_image_materialization_jobs) AND NOT EXISTS(SELECT 1 FROM repositories WHERE settings->'agent_runs_enabled' IS DISTINCT FROM 'false'::jsonb)",
    ).fetch_one(pool).await?;
    if !controlled {
        return Err(held("unexpected ordinary admission or service work"));
    }
    let commands: Vec<Command> = sqlx::query_as(
        "SELECT command_key,operation,aggregate_id,secondary_id,actor_id,request_id,input_hash FROM release_command_inbox ORDER BY command_key",
    ).fetch_all(pool).await?;
    if commands.len() != 2 {
        return Err(held("unexpected release command history"));
    }
    require_commands(pair, &commands)?;
    let migrations: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE success ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    let expected = sqlx::migrate!("../../migrations")
        .iter()
        .map(|m| (m.version, m.checksum.to_vec()))
        .collect::<Vec<_>>();
    if migrations != expected || migrations.len() != 107 {
        return Err(held("public107 history changed"));
    }
    let vm_ids: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT build_request_id,vm_id FROM build_executions ORDER BY build_request_id",
    )
    .fetch_all(pool)
    .await?;
    let expected = pair
        .each_ref()
        .map(|p| VmId(format!("build-{}", p.published.build_request_id)));
    for (id, vm) in &vm_ids {
        if !pair
            .iter()
            .zip(&expected)
            .any(|(p, expected)| p.published.build_request_id == *id && expected.0 == *vm)
        {
            return Err(held("actual build VM identity differs"));
        }
    }
    let isolated = open_directory(&pins[0].directory, "isolated-builds")?;
    let active = open_directory(&isolated, "active")?;
    Ok(PublicationCheckpoint {
        isolated,
        active,
        expected,
        state: json!([inventory, commands, migrations, vm_ids]),
    })
}

fn require_commands(
    pair: &[std::sync::Arc<PublishedVariant>; 2],
    commands: &[Command],
) -> Result<(), BuildError> {
    for p in pair {
        let key = ReleaseCommandKey::derive(
            "complete-isolated-build",
            &[p.published.build_request_id.as_bytes()],
        );
        if !commands.iter().any(|c| {
            c.0.as_slice() == key.as_bytes().as_slice()
                && c.1 == "complete_build"
                && c.2 == p.published.release_id
                && c.3 == Some(p.published.release_agent_id)
                && c.4.is_none()
                && c.5.is_none()
                && c.6.is_none()
        }) {
            return Err(held("genuine complete_build command identity differs"));
        }
    }
    Ok(())
}

impl PublicationCheckpoint {
    pub(in super::super) fn matches(&self, other: &Self) -> Result<bool, BuildError> {
        Ok(self.expected == other.expected
            && self.state == other.state
            && file_identity(&self.isolated)? == file_identity(&other.isolated)?
            && file_identity(&self.active)? == file_identity(&other.active)?)
    }
}

pub(in super::super) fn require_workspaces_absent(
    pair: &[std::sync::Arc<PublishedVariant>; 2],
    checkpoint: &PublicationCheckpoint,
) -> Result<(), BuildError> {
    let active = &checkpoint.active;
    for p in pair {
        match rustix::fs::statat(
            active,
            p.published.build_request_id.to_string(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(held("actual build workspace remains after joins")),
        }
    }
    for entry in rustix::fs::Dir::read_from(active)? {
        if !matches!(entry?.file_name().to_bytes(), b"." | b"..") {
            return Err(held("unexpected active build workspace"));
        }
    }
    Ok(())
}

fn held(reason: &'static str) -> BuildError {
    std::io::Error::other(reason).into()
}
