//! Ordinary filesystem observations exercise failure gates, not native drain.
use super::{Cgroup, parse_populated};
use std::{fs, os::unix::fs::symlink, time::Instant};
use tempfile::TempDir;

fn strict_cgroup(temp: &TempDir) -> Cgroup {
    let path = temp.path().join("vm");
    fs::create_dir(&path).unwrap();
    Cgroup {
        path,
        emulated: false,
    }
}

#[test]
fn populated_requires_one_exact_zero_or_one_observation() {
    assert!(!parse_populated("populated 0\nfrozen 0\n").unwrap());
    assert!(parse_populated("frozen 0\npopulated 1\n").unwrap());
    for invalid in [
        "",
        "frozen 0\n",
        "populated\n",
        "populated 2\n",
        "populated -1\n",
        "populated 01\n",
        "populated 0 extra\n",
        "populated 0\npopulated 1\n",
        "populated 0\npopulated 0\n",
        "populated 0\nfrozen invalid\n",
        "populated 0\nfrozen 0\nfrozen 0\n",
        "populated 0\n\n",
    ] {
        assert!(parse_populated(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn existing_cgroup_never_treats_failed_events_observation_as_empty() {
    let temp = TempDir::new().unwrap();
    let cgroup = strict_cgroup(&temp);
    let events = cgroup.path.join("cgroup.events");
    assert!(
        cgroup
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("cgroup-events")
    );
    assert!(cgroup.path.is_dir());
    fs::create_dir(&events).unwrap();
    assert!(
        cgroup
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("cgroup-events")
    );
    fs::remove_dir(&events).unwrap();
    fs::write(&events, [0xff]).unwrap();
    assert!(
        cgroup
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("cgroup-events")
    );
    fs::write(&events, "populated invalid\n").unwrap();
    assert!(
        cgroup
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("cgroup-events")
    );
    assert_eq!(fs::read_to_string(events).unwrap(), "populated invalid\n");
}

#[test]
fn populated_at_deadline_is_an_explicit_failure_before_removal() {
    let temp = TempDir::new().unwrap();
    let cgroup = strict_cgroup(&temp);
    fs::write(cgroup.path.join("cgroup.events"), "populated 1\n").unwrap();
    let error = cgroup.wait_until_empty(Instant::now()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("still populated at cleanup deadline")
    );
    assert!(cgroup.path.is_dir());
    fs::write(cgroup.path.join("cgroup.events"), "populated 0\n").unwrap();
    cgroup.wait_until_empty(Instant::now()).unwrap();
}

#[test]
fn only_exact_child_absence_is_idempotent_and_symlinks_are_denied() {
    let temp = TempDir::new().unwrap();
    let absent = Cgroup {
        path: temp.path().join("absent"),
        emulated: false,
    };
    absent.cleanup().unwrap();
    let missing_parent = Cgroup {
        path: temp.path().join("missing-parent/vm"),
        emulated: false,
    };
    assert!(missing_parent.cleanup().is_err());
    let foreign = temp.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("cgroup.kill"), b"foreign bytes").unwrap();
    symlink(&foreign, &absent.path).unwrap();
    assert!(absent.cleanup().is_err());
    assert_eq!(
        fs::read(foreign.join("cgroup.kill")).unwrap(),
        b"foreign bytes"
    );
}

#[test]
fn redirected_controls_are_denied_without_foreign_io() {
    let temp = TempDir::new().unwrap();
    let cgroup = strict_cgroup(&temp);
    let foreign = temp.path().join("foreign-control");
    fs::write(&foreign, "populated 0\n").unwrap();
    let events = cgroup.path.join("cgroup.events");
    symlink(&foreign, &events).unwrap();
    assert!(cgroup.cleanup().is_err());
    fs::remove_file(&events).unwrap();
    fs::write(&events, "populated 0\n").unwrap();
    symlink(&foreign, cgroup.path.join("cgroup.kill")).unwrap();
    assert!(cgroup.cleanup().is_err());
    assert_eq!(fs::read_to_string(foreign).unwrap(), "populated 0\n");
}

#[test]
fn kill_request_never_turns_persistent_population_into_cleanup_success() {
    let temp = TempDir::new().unwrap();
    let cgroup = strict_cgroup(&temp);
    let events = cgroup.path.join("cgroup.events");
    let kill = cgroup.path.join("cgroup.kill");
    fs::write(&events, "populated 1\n").unwrap();
    fs::write(&kill, "").unwrap();
    let error = cgroup.cleanup().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("still populated at cleanup deadline")
    );
    assert_eq!(fs::read_to_string(kill).unwrap(), "1");
    assert_eq!(fs::read_to_string(events).unwrap(), "populated 1\n");
    assert!(cgroup.path.is_dir());
}
