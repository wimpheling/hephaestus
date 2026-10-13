use super::{fixture, original::DeleteFixture};
use crate::{
    DeploymentAdmission, DeploymentError, PreparedInstanceDeleteHistory,
    PreparedInstanceDetachRecovery, RemoveContinuation, RemoveResumeAdmission, ResourceAction,
};

fn qualify(
    exact: &DeleteFixture,
    admission: DeploymentAdmission,
    active: Vec<RemoveContinuation>,
    detach: Vec<PreparedInstanceDetachRecovery>,
    terminal: Vec<PreparedInstanceDeleteHistory>,
) -> Result<RemoveResumeAdmission, DeploymentError> {
    RemoveResumeAdmission::with_instance_terminal_history(
        admission,
        active,
        vec![exact.closure.clone()],
        vec![],
        detach,
        terminal,
        &exact.configuration,
    )
}

#[test]
fn fresh_resume_reaches_active_detach_recovery_and_active_delete_without_old_fallback() {
    let exact = fixture::exact(ResourceAction::Detach);
    let observed = fixture::progress(&exact.effect);
    let recovery =
        PreparedInstanceDetachRecovery::new(fixture::detach(&exact), observed.clone()).unwrap();
    let admitted = qualify(
        &exact,
        fixture::admission(&exact, &observed),
        vec![fixture::active(&exact)],
        vec![recovery.clone()],
        vec![],
    )
    .unwrap();
    assert_eq!(admitted.instance_detach_recovery(), &[recovery]);
    assert_eq!(admitted.continuations(), &[fixture::active(&exact)]);
    let exact = fixture::exact(ResourceAction::Delete);
    let observed = fixture::progress(&exact.effect);
    let active = fixture::active(&exact);
    let admission = fixture::admission(&exact, &observed);
    assert!(
        RemoveResumeAdmission::with_complete_closures(
            admission.clone(),
            vec![active.clone()],
            vec![exact.closure.clone()]
        )
        .is_err()
    );
    let terminal = PreparedInstanceDeleteHistory::from_observed_parts(
        fixture::delete(&exact),
        observed,
        None,
        None,
        None,
    )
    .unwrap();
    let admitted = qualify(
        &exact,
        admission,
        vec![active.clone()],
        vec![],
        vec![terminal.clone()],
    )
    .unwrap();
    assert_eq!(admitted.instance_delete_history(), &[terminal]);
    assert_eq!(admitted.continuations(), &[active]);
}

#[test]
fn inactive_applied_history_survives_fresh_manager_without_an_active_claim() {
    let exact = fixture::exact(ResourceAction::Delete);
    let observed = fixture::applied(&exact.effect);
    let terminal = fixture::committed(&exact, observed.clone()).unwrap();
    let admission = fixture::admission(&exact, &observed);
    let admitted = qualify(
        &exact,
        admission.clone(),
        vec![],
        vec![],
        vec![terminal.clone()],
    )
    .unwrap();
    assert!(admitted.continuations().is_empty());
    assert_eq!(admitted.instance_delete_history(), &[terminal.clone()]);
    assert!(
        qualify(
            &exact,
            admission,
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal]
        )
        .is_err()
    );
}

#[test]
fn every_active_snapshot_attempt_and_terminal_entry_must_be_accounted_once() {
    let exact = fixture::exact(ResourceAction::Delete);
    let observed = fixture::progress(&exact.effect);
    let terminal = fixture::committed(&exact, observed.clone()).unwrap();
    let admission = fixture::admission(&exact, &observed);
    assert!(
        qualify(
            &exact,
            admission.clone(),
            vec![],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    assert!(
        qualify(
            &exact,
            admission.clone(),
            vec![fixture::active(&exact)],
            vec![],
            vec![]
        )
        .is_err()
    );
    assert!(
        qualify(
            &exact,
            admission.clone(),
            vec![fixture::active(&exact), fixture::active(&exact)],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    assert!(
        qualify(
            &exact,
            admission.clone(),
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal.clone(), terminal.clone()]
        )
        .is_err()
    );
    let mut extra = admission.clone();
    let (_, state) = extra
        .snapshot
        .resources
        .iter_mut()
        .find(|(name, _)| **name != exact.effect.resource)
        .unwrap();
    state.active_attempt = Some(super::original::attempt(999));
    assert!(
        qualify(
            &exact,
            extra,
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    let mut extra_claim = fixture::active(&exact);
    let mut extra_admission = admission.clone();
    let (name, state) = extra_admission
        .snapshot
        .resources
        .iter_mut()
        .find(|(name, _)| **name != exact.effect.resource)
        .unwrap();
    extra_claim.claim.resource = name.clone();
    extra_claim.claim.provenance.attempt_id = super::original::attempt(998);
    state.active_attempt = Some(extra_claim.claim.provenance.attempt_id);
    state.version = extra_claim.claim.resource_version;
    assert!(
        qualify(
            &exact,
            extra_admission,
            vec![fixture::active(&exact), extra_claim],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    let mut missing = admission;
    missing.snapshot.resources.remove(&exact.effect.resource);
    assert!(
        qualify(
            &exact,
            missing,
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal]
        )
        .is_err()
    );
}

#[test]
fn progress_current_command_configuration_and_tag_substitutions_deny() {
    let exact = fixture::exact(ResourceAction::Delete);
    let observed = fixture::progress(&exact.effect);
    let terminal = fixture::committed(&exact, observed.clone()).unwrap();
    let admission = fixture::admission(&exact, &observed);
    let mut changed = admission.clone();
    changed
        .snapshot
        .resources
        .get_mut(&exact.effect.resource)
        .unwrap()
        .version += 1;
    assert!(
        qualify(
            &exact,
            changed,
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    let mut changed = admission.clone();
    changed.receipt.command = super::original::fresh_command();
    assert!(
        qualify(
            &exact,
            changed,
            vec![fixture::active(&exact)],
            vec![],
            vec![terminal.clone()]
        )
        .is_err()
    );
    let mut foreign = fixture::exact(ResourceAction::Delete);
    foreign.change_owner("other-owner", "host-a");
    let foreign = fixture::committed(&foreign, observed.clone()).unwrap();
    assert!(
        qualify(
            &exact,
            admission.clone(),
            vec![fixture::active(&exact)],
            vec![],
            vec![foreign]
        )
        .is_err()
    );
    let mut wrong = fixture::active(&exact);
    wrong.closure.as_mut().unwrap().request = super::super::fixture::identity(17, 18).request_id;
    assert!(qualify(&exact, admission, vec![wrong], vec![], vec![terminal]).is_err());
    let mut tagged = fixture::exact(ResourceAction::Delete);
    tagged.fresh_closure();
    let terminal = fixture::committed(&tagged, observed.clone()).unwrap();
    qualify(
        &tagged,
        fixture::admission(&tagged, &observed),
        vec![fixture::active(&tagged)],
        vec![],
        vec![terminal],
    )
    .unwrap();
}

#[test]
fn detach_and_delete_inventories_cannot_share_a_resource_or_mask_recovery_lifecycle() {
    let exact = fixture::exact(ResourceAction::Delete);
    let observed = fixture::progress(&exact.effect);
    let terminal = fixture::committed(&exact, observed.clone()).unwrap();
    let old = fixture::exact(ResourceAction::Detach);
    let recovery =
        PreparedInstanceDetachRecovery::new(fixture::detach(&old), fixture::progress(&old.effect))
            .unwrap();
    assert!(
        qualify(
            &exact,
            fixture::admission(&exact, &observed),
            vec![fixture::active(&exact)],
            vec![recovery],
            vec![terminal]
        )
        .is_err()
    );
    let observed = fixture::progress(&old.effect);
    let recovery =
        PreparedInstanceDetachRecovery::new(fixture::detach(&old), observed.clone()).unwrap();
    let mut wrong = fixture::admission(&old, &observed);
    wrong.snapshot.lifecycle = crate::DeploymentLifecycle::Removing;
    assert!(
        qualify(
            &old,
            wrong,
            vec![fixture::active(&old)],
            vec![recovery],
            vec![]
        )
        .is_err()
    );
}
