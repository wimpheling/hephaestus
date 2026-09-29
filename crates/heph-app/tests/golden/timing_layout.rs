use super::*;

pub const WORKLOAD_PHASE_TIMING_EVENT: &str = "phase-timing";
pub const WORKLOAD_PHASE_TIMING_MAX_MS: u128 = 45 * 60 * 1_000;

/// Reports bounded timings from the workload process. The trusted supervisor
/// remains authoritative for test outcomes and acceptance decisions.
pub struct WorkloadPhaseTimer {
    phase: &'static str,
    started: Instant,
    enabled: bool,
    finished: bool,
}

impl WorkloadPhaseTimer {
    pub fn start(phase: &'static str, enabled: bool) -> Self {
        Self {
            phase,
            started: Instant::now(),
            enabled,
            finished: false,
        }
    }

    pub fn finish(mut self, success: bool) {
        self.finished = true;
        self.emit(if success { "passed" } else { "failed" });
    }

    pub fn emit(&self, status: &'static str) {
        if !self.enabled {
            return;
        }
        let elapsed_ms = self.started.elapsed().as_millis();
        let duration_ms = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            elapsed_ms
        } else {
            0
        };
        let status = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            status
        } else {
            "unknown"
        };
        eprintln!(
            "HEPH_GCP_COOKING event={WORKLOAD_PHASE_TIMING_EVENT} phase={} status={status} duration_ms={duration_ms}",
            self.phase
        );
    }
}

impl Drop for WorkloadPhaseTimer {
    fn drop(&mut self) {
        if !self.finished {
            self.emit("unknown");
        }
    }
}

pub fn cooking_base_layout(
    layouts: &BTreeMap<String, PathBuf>,
    reference: &str,
) -> Option<PathBuf> {
    let (name, digest) = reference.rsplit_once('@')?;
    let key = name.rsplit('/').next()?;
    let suffix = format!("/{key}@{digest}");
    layouts
        .iter()
        .find(|(candidate, _)| candidate.ends_with(&suffix))
        .map(|(_, path)| path.clone())
}

pub fn cooking_base_layout_mount_roots(
    layouts: &BTreeMap<String, PathBuf>,
) -> Result<Vec<PathBuf>, (PathBuf, std::io::Error)> {
    let mut roots = layouts
        .values()
        .map(|path| path.canonicalize().map_err(|error| (path.clone(), error)))
        .collect::<Result<Vec<_>, _>>()?;
    roots.sort_unstable();
    roots.dedup();
    Ok(roots)
}

#[test]
pub fn cooking_base_layout_alias_requires_identical_image_digest() {
    let reviewed = format!(
        "registry.invalid/platform/python-ubuntu@sha256:{}",
        "a".repeat(64)
    );
    let layouts = BTreeMap::from([(reviewed, PathBuf::from("/reviewed/python"))]);
    let same = format!("localhost/python-ubuntu@sha256:{}", "a".repeat(64));
    let different = format!("localhost/python-ubuntu@sha256:{}", "b".repeat(64));
    assert_eq!(
        cooking_base_layout(&layouts, &same),
        Some(PathBuf::from("/reviewed/python"))
    );
    assert_eq!(cooking_base_layout(&layouts, &different), None);
}

#[test]
pub fn cooking_base_layout_mount_roots_are_canonical_and_exact() {
    let temporary = tempfile::tempdir().expect("golden temporary root");
    let python = temporary.path().join("release/python/image");
    let rust = temporary.path().join("release/rust/image");
    std::fs::create_dir_all(&python).expect("python layout directory");
    std::fs::create_dir_all(&rust).expect("rust layout directory");
    let alias = temporary.path().join("release/python/../python/image");
    let layouts = BTreeMap::from([
        (String::from("python-a"), alias),
        (String::from("python-b"), python.clone()),
        (String::from("rust"), rust.clone()),
    ]);

    assert_eq!(
        cooking_base_layout_mount_roots(&layouts).expect("canonical layout roots"),
        vec![
            python.canonicalize().expect("canonical python layout"),
            rust
        ]
    );
    let missing = BTreeMap::from([(String::from("missing"), temporary.path().join("absent"))]);
    assert!(cooking_base_layout_mount_roots(&missing).is_err());
}

pub fn workload_phase_timing_from_environment() -> bool {
    env::var_os("HEPH_GCP_PHASE_TIMING_PATH").is_some()
}
