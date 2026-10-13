//! Scheduling/comparison tests only, not `PostgreSQL` or physical-stop proof.

#[path = "models.rs"]
mod models;
#[path = "ports.rs"]
mod ports;

use super::{compare, expired};
use models::{generic, git};
use ports::Ports;
use runtime_authority::{RuntimeAuthorityError, RuntimeSessionIssuer};
use runtime_git_authority::{RuntimeGitAuthorityError, RuntimeGitCredentialIssuer};
use std::{
    future::Future,
    sync::atomic::Ordering,
    task::{Context, Poll, Waker},
};

fn complete<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    let result = future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    drop(future);
    match result {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("controlled ports unexpectedly yielded"),
    }
}
fn execute(ports: &Ports) -> Result<(), heph_run::RunAuthorityError> {
    let original = generic(0);
    let issuer = RuntimeSessionIssuer::new(ports.clone(), ports.clone());
    let git_issuer = RuntimeGitCredentialIssuer::new(ports.clone(), ports.clone());
    complete(expired(
        &issuer,
        &git_issuer,
        original.pins().run(),
        original.session().id,
        original.session().generation,
    ))
}

#[test]
fn both_observations_precede_either_delete_for_present_and_positive_absence() {
    for present in [false, true] {
        let original = generic(0);
        let git_original = git(&original, present);
        assert_ne!(original.observed_at(), git_original.ceiling().observed_at());
        let ports = Ports::new(original.clone(), git_original.clone());
        execute(&ports).unwrap();
        assert_eq!(
            *ports.log.lock().unwrap(),
            vec![
                "observe generic",
                "observe git",
                "delete generic",
                "delete git"
            ]
        );
        assert_eq!(*ports.deleted_generic.lock().unwrap(), vec![original]);
        assert_eq!(*ports.deleted_git.lock().unwrap(), vec![git_original]);
    }
}

#[test]
fn either_observation_failure_blocks_both_deletes_without_fallback() {
    for generic_failure in [false, true] {
        let original = generic(0);
        let mut ports = Ports::new(original.clone(), git(&original, false));
        if generic_failure {
            ports.generic = Err(RuntimeAuthorityError::Unsupported);
        } else {
            ports.git = Err(RuntimeGitAuthorityError::Unsupported);
        }
        assert!(execute(&ports).is_err());
        let expected = if generic_failure {
            vec!["observe generic"]
        } else {
            vec!["observe generic", "observe git"]
        };
        assert_eq!(*ports.log.lock().unwrap(), expected);
        assert!(ports.deleted_generic.lock().unwrap().is_empty());
        assert!(ports.deleted_git.lock().unwrap().is_empty());
    }
}

#[test]
fn coherent_cross_ceiling_substitutions_all_deny_before_deletion() {
    let original = generic(0);
    for change in 1..=8 {
        let changed = git(&generic(change), false);
        assert!(compare(&original, &changed).is_err());
        let ports = Ports::new(original.clone(), changed);
        assert!(execute(&ports).is_err());
        assert_eq!(
            *ports.log.lock().unwrap(),
            vec!["observe generic", "observe git"]
        );
        assert!(ports.deleted_generic.lock().unwrap().is_empty());
        assert!(ports.deleted_git.lock().unwrap().is_empty());
    }
}

#[test]
fn generic_delete_failure_preserves_primary_error_and_never_deletes_git() {
    let original = generic(0);
    let ports = Ports::new(original.clone(), git(&original, true));
    ports.fail_generic.store(true, Ordering::SeqCst);
    let error = execute(&ports).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&RuntimeAuthorityError::HandoffUnavailable.to_string())
    );
    assert_eq!(
        *ports.log.lock().unwrap(),
        vec!["observe generic", "observe git", "delete generic"]
    );
    assert!(ports.deleted_git.lock().unwrap().is_empty());
}

#[test]
fn partial_git_failure_retains_original_ports_and_retry_requalifies_both() {
    let original = generic(0);
    let git_original = git(&original, true);
    let ports = Ports::new(original.clone(), git_original.clone());
    {
        let issuer = RuntimeSessionIssuer::new(ports.clone(), ports.clone());
        let git_issuer = RuntimeGitCredentialIssuer::new(ports.clone(), ports.clone());
        ports.fail_git.store(true, Ordering::SeqCst);
        let attempt = || {
            expired(
                &issuer,
                &git_issuer,
                original.pins().run(),
                original.session().id,
                original.session().generation,
            )
        };
        let error = complete(attempt()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&RuntimeGitAuthorityError::HandoffUnavailable.to_string())
        );
        ports.fail_git.store(false, Ordering::SeqCst);
        complete(attempt()).unwrap();
    }
    assert_eq!(
        *ports.log.lock().unwrap(),
        vec![
            "observe generic",
            "observe git",
            "delete generic",
            "delete git",
            "observe generic",
            "observe git",
            "delete generic",
            "delete git"
        ]
    );
    assert_eq!(
        *ports.deleted_generic.lock().unwrap(),
        vec![original.clone(), original]
    );
    assert_eq!(
        *ports.deleted_git.lock().unwrap(),
        vec![git_original.clone(), git_original]
    );
}
