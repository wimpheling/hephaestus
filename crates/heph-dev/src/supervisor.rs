use crate::{
    build,
    cli::BuildSelection,
    context::DevContext,
    process::{DevError, Result, run_quiet},
    zot,
};
use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

pub fn run(context: &DevContext, watch: bool) -> Result<()> {
    refuse_duplicate_supervisor(context)?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupt_handler = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        interrupt_handler.store(true, Ordering::SeqCst);
    })
    .map_err(|error| DevError::Invalid(format!("could not install Ctrl-C handler: {error}")))?;
    zot::start(context)?;

    let script = context.repository_root.join("scripts/run-local.sh");
    let mut child = match Command::new(script)
        .current_dir(&context.repository_root)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ignored = zot::stop(context);
            return Err(error.into());
        }
    };
    let result = if watch {
        println!("Rust watch mode enabled; Phoenix development watchers remain active");
        supervise_with_watch(context, &mut child, &interrupted)
    } else {
        supervise(context, &mut child, &interrupted)
    };
    let _ignored = zot::stop(context);
    result
}

fn supervise(_context: &DevContext, child: &mut Child, interrupted: &AtomicBool) -> Result<()> {
    let mut termination_sent = false;
    let mut interrupted_at = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return finish(status, interrupted.load(Ordering::SeqCst));
        }
        handle_interrupt(
            child,
            interrupted,
            &mut interrupted_at,
            &mut termination_sent,
        )?;
        thread::sleep(Duration::from_millis(100));
    }
}

fn supervise_with_watch(
    context: &DevContext,
    child: &mut Child,
    interrupted: &AtomicBool,
) -> Result<()> {
    let mut snapshot = source_snapshot(context);
    let mut termination_sent = false;
    let mut interrupted_at = None;
    loop {
        if let Some(status) = child.try_wait()? {
            return finish(status, interrupted.load(Ordering::SeqCst));
        }
        if interrupted.load(Ordering::SeqCst) {
            handle_interrupt(
                child,
                interrupted,
                &mut interrupted_at,
                &mut termination_sent,
            )?;
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let current = source_snapshot(context);
        if current != snapshot {
            thread::sleep(Duration::from_millis(300));
            snapshot = source_snapshot(context);
            println!("Rust source changed; rebuilding daemon and runtime");
            match build::build(context, &BuildSelection::rust_only()) {
                Ok(()) => {
                    println!("build succeeded; restarting the local daemon");
                    signal_daemon_restart(child)?;
                }
                Err(error) => {
                    eprintln!("watch build failed; the previous daemon remains active: {error}");
                }
            }
        }
        thread::sleep(Duration::from_millis(400));
    }
}

fn handle_interrupt(
    child: &Child,
    interrupted: &AtomicBool,
    interrupted_at: &mut Option<Instant>,
    termination_sent: &mut bool,
) -> Result<()> {
    if !interrupted.load(Ordering::SeqCst) {
        return Ok(());
    }
    let started = interrupted_at.get_or_insert_with(Instant::now);
    if !*termination_sent && started.elapsed() >= Duration::from_secs(30) {
        eprintln!("graceful shutdown timed out; terminating the supervisor");
        terminate(child)?;
        *termination_sent = true;
    }
    Ok(())
}

fn refuse_duplicate_supervisor(context: &DevContext) -> Result<()> {
    let Ok(pid) = fs::read_to_string(context.supervisor_pid_file()) else {
        return Ok(());
    };
    let pid = pid.trim();
    if !pid.is_empty() && run_quiet("kill", &["-0", pid])? {
        return Err(DevError::SupervisorActive);
    }
    Ok(())
}

fn terminate(child: &Child) -> Result<()> {
    let pid = child.id().to_string();
    if run_quiet("kill", &["-TERM", &pid])? {
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "could not terminate development supervisor {pid}"
        )))
    }
}

fn signal_daemon_restart(child: &Child) -> Result<()> {
    let pid = child.id().to_string();
    if run_quiet("kill", &["-USR1", &pid])? {
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "could not signal development supervisor {pid}"
        )))
    }
}

fn finish(status: std::process::ExitStatus, interrupted: bool) -> Result<()> {
    if status.success() || interrupted {
        Ok(())
    } else {
        Err(DevError::Command {
            program: "scripts/run-local.sh".into(),
            status,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceSnapshot {
    files: u64,
    bytes: u64,
    latest: Option<SystemTime>,
    fingerprint: u64,
}

fn source_snapshot(context: &DevContext) -> SourceSnapshot {
    let mut snapshot = SourceSnapshot {
        files: 0,
        bytes: 0,
        latest: None,
        fingerprint: 0,
    };
    for path in [
        context.repository_root.join("Cargo.toml"),
        context.repository_root.join("Cargo.lock"),
        context.repository_root.join("crates"),
        context.repository_root.join("migrations"),
    ] {
        collect_snapshot(&path, &mut snapshot);
    }
    snapshot
}

fn collect_snapshot(path: &Path, snapshot: &mut SourceSnapshot) {
    let Ok(metadata) = path.symlink_metadata() else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        if watched_file(path) {
            snapshot.files = snapshot.files.saturating_add(1);
            snapshot.bytes = snapshot.bytes.saturating_add(metadata.len());
            let mut hasher = DefaultHasher::new();
            path.hash(&mut hasher);
            metadata.len().hash(&mut hasher);
            if let Ok(modified) = metadata.modified() {
                modified.hash(&mut hasher);
                snapshot.latest = Some(
                    snapshot
                        .latest
                        .map_or(modified, |latest| latest.max(modified)),
                );
            }
            snapshot.fingerprint ^= hasher.finish();
        }
        return;
    }
    let Ok(entries) = path.read_dir() else {
        return;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        collect_snapshot(&entry.path(), snapshot);
    }
}

fn watched_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "toml" | "sql")
    )
}

#[cfg(test)]
mod tests {
    use super::{SourceSnapshot, collect_snapshot};
    use std::{fs, time::SystemTime};
    use tempfile::tempdir;

    #[test]
    fn snapshots_only_watched_source_files() {
        let fixture = tempdir().expect("fixture");
        fs::write(fixture.path().join("lib.rs"), "fn main() {}").expect("Rust fixture");
        fs::write(fixture.path().join("notes.txt"), "ignored").expect("text fixture");
        let mut snapshot = SourceSnapshot {
            files: 0,
            bytes: 0,
            latest: Some(SystemTime::UNIX_EPOCH),
            fingerprint: 0,
        };
        collect_snapshot(fixture.path(), &mut snapshot);
        assert_eq!(snapshot.files, 1);
        assert_eq!(snapshot.bytes, 12);
    }
}
