use crate::common::{AgentConfig, AgentInstanceId, RepositoryId, RunId, Serialize};

pub struct RunRequest {
    pub repository_id: RepositoryId,
    pub commit: String,
    pub instance_id: AgentInstanceId,
    pub config: AgentConfig,
}

pub struct Materialized {
    pub tree: String,
    pub manifest_hash: String,
}

pub struct Imported {
    pub tree: String,
    pub commit: String,
    pub manifest: Vec<u8>,
    pub manifest_hash: String,
    pub patch: Vec<u8>,
    pub declared_files: Vec<DeclaredFile>,
}

pub struct ImportRequest {
    pub input_commit: String,
    pub message: String,
    pub timestamp: i64,
    pub declared_paths: Vec<String>,
    pub repository_id: RepositoryId,
    pub run_id: RunId,
}

pub struct DeclaredFile {
    pub path: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

#[derive(Serialize)]
pub struct Manifest {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_commit: Option<String>,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Serialize)]
pub struct ManifestEntry {
    pub path: String,
    pub kind: &'static str,
    pub mode: u32,
    pub size: u64,
    pub sha256: String,
}

#[derive(Default)]
pub struct ImportCounters {
    pub entries: usize,
    pub bytes: u64,
}

pub struct TreeObject {
    pub mode: u32,
    pub kind: &'static str,
    pub object_id: String,
    pub name: String,
}
