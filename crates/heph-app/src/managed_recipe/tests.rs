#[path = "tests/budget.rs"]
mod budget;
#[path = "tests/shutdown.rs"]
mod shutdown;
// Controlled actual request/task/close protocol; no catalog, DB, VM or Ready proof.
use super::{Actions, Arc, Failure, ManagedRecipe, Outcome, Request};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{CommandIdentity, DeploymentKey, DeploymentOperation, PlanningRequest};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Semaphore, mpsc};
struct Event {
    request: RequestId,
    resume: bool,
}
struct Owner(Arc<AtomicBool>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct Controlled {
    entered: mpsc::UnboundedSender<Event>,
    release: Arc<Semaphore>,
    calls: Arc<AtomicUsize>,
    panic_call: Option<usize>,
    denial_call: Option<usize>,
    _owner: Owner,
}
#[async_trait::async_trait]
impl Actions for Controlled {
    async fn run(&self, request: &Request) -> Outcome {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered
            .send(Event {
                request: request.identity().request_id,
                resume: matches!(request, Request::Resume { .. }),
            })
            .unwrap();
        assert_ne!(
            self.panic_call,
            Some(call),
            "controlled original recipe operation panic"
        );
        if self.denial_call == Some(call) {
            return Err(Arc::new(Failure::Execution(
                crate::recipe_engine::RecipeApplicationError::Deployment(
                    recipe_application::DeploymentError::AuthorizationDenied,
                ),
            )));
        }
        self.release.acquire().await.unwrap().forget();
        Err(Arc::new(Failure::Unsettled))
    }
}
struct Control {
    entered: mpsc::UnboundedReceiver<Event>,
    release: Arc<Semaphore>,
    calls: Arc<AtomicUsize>,
    owner_dropped: Arc<AtomicBool>,
}
fn service(panic_first: bool) -> (Arc<ManagedRecipe>, Control) {
    controlled_service(panic_first.then_some(0), None)
}
fn controlled_service(
    panic_call: Option<usize>,
    denial_call: Option<usize>,
) -> (Arc<ManagedRecipe>, Control) {
    let (send, entered) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let owner_dropped = Arc::new(AtomicBool::new(false));
    let actions = Arc::new(Controlled {
        entered: send,
        release: release.clone(),
        calls: calls.clone(),
        panic_call,
        denial_call,
        _owner: Owner(owner_dropped.clone()),
    });
    (
        Arc::new(ManagedRecipe {
            registry: Weak::new(),
            actions,
            jobs: Mutex::default(),
        }),
        Control {
            entered,
            release,
            calls,
            owner_dropped,
        },
    )
}
pub(super) fn identity() -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        UserId::new(),
        "https://fixture.example",
        "verified-subject",
        serde_json::json!({}),
        RequestId::new(),
    )
}
pub(super) fn plan() -> PlanningRequest {
    PlanningRequest::new(
        forge_domain::ProjectId::from_uuid(uuid::Uuid::from_u128(1)),
        DeploymentKey::parse("controlled-deployment").unwrap(),
        br#"contract_version = 1
recipe_id = "application"
recipe_version = "1.0.0"
[[resources]]
kind = "instance"
name = "app"
removal = "delete"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
"#,
        BTreeMap::new(), BTreeMap::new(),
    ).unwrap()
}
#[tokio::test]
async fn caller_abort_keeps_actual_request_owner_and_held_operation_until_real_join() {
    let (service, mut control) = service(false);
    let actor = identity();
    let request_id = actor.request_id;
    let ticket = service
        .submit(Request::install(actor, plan()).unwrap())
        .unwrap();
    let waiter = tokio::spawn(ticket.wait());
    assert_eq!(control.entered.recv().await.unwrap().request, request_id);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert!(!control.owner_dropped.load(Ordering::SeqCst));
    {
        let state = service.state();
        let entry = &state.entries[&1];
        assert_eq!(entry.request.identity().request_id, request_id);
        assert!(entry.outcome.is_none());
        assert!(!entry.joined);
        drop(state);
    }
    control.release.add_permits(1);
    assert!(service.join_jobs().await.is_err());
    {
        let state = service.state();
        assert!(state.entries[&1].joined);
        assert!(state.entries[&1].outcome.as_ref().unwrap().is_err());
        drop(state);
    }
    drop(service);
    assert!(control.owner_dropped.load(Ordering::SeqCst));
}
#[tokio::test]
async fn original_resume_is_fifo_and_fresh_request_keeps_the_install_logical_identity() {
    let (service, mut control) = service(false);
    let actor = identity();
    let request = plan();
    let original = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let first = service
        .submit(Request::install(actor.clone(), request.clone()).unwrap())
        .unwrap();
    let mut fresh = actor.clone();
    fresh.request_id = RequestId::new();
    let second_id = fresh.request_id;
    let second = service
        .submit(Request::resume(fresh, original, request.id(), plan().project_id()).unwrap())
        .unwrap();
    let entered = control.entered.recv().await.unwrap();
    assert!(!entered.resume);
    assert_eq!(entered.request, actor.request_id);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    control.release.add_permits(1);
    assert!(first.wait().await.is_err());
    let resumed = control.entered.recv().await.unwrap();
    assert!(resumed.resume);
    assert_eq!(resumed.request, second_id);
    control.release.add_permits(1);
    assert!(second.wait().await.is_err());
    assert!(service.join_jobs().await.is_err());
    {
        let state = service.state();
        assert!(state.entries.values().all(|entry| entry.joined));
        assert_eq!(state.entries[&2].request.original(), original);
        drop(state);
    }
    drop(service);
    assert!(control.owner_dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn synchronous_close_denies_late_mutation_but_does_not_cancel_admitted_future() {
    let (service, mut control) = service(false);
    let actor = identity();
    let request = plan();
    let original = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let admitted = service
        .submit(Request::install(actor.clone(), request.clone()).unwrap())
        .unwrap();
    service.close();
    assert!(
        service
            .submit(Request::resume(actor, original, request.id(), plan().project_id()).unwrap())
            .is_err()
    );
    assert!(!control.entered.recv().await.unwrap().resume);
    control.release.add_permits(1);
    assert!(admitted.wait().await.is_err());
    assert!(service.join_jobs().await.is_err());
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    drop(service);
}

#[tokio::test]
async fn panic_keeps_original_join_error_and_does_not_authorize_replacement_install() {
    let (service, mut control) = service(true);
    let first = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    let replacement = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    let _actual_entry = control.entered.recv().await.unwrap();
    assert!(first.wait().await.is_err());
    assert!(replacement.wait().await.is_err());
    let later = service
        .submit(Request::install(identity(), plan()).unwrap())
        .unwrap();
    assert!(later.wait().await.is_err());
    let error = service.join_jobs().await.unwrap_err();
    assert!(matches!(error.as_ref(), Failure::Join(cause) if cause.is_panic()));
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert!(!control.owner_dropped.load(Ordering::SeqCst));
    {
        let state = service.state();
        assert!(state.entries[&1].join_error.is_some());
        assert!(state.entries.values().all(|entry| entry.joined));
        drop(state);
    }
    drop(service);
    assert!(control.owner_dropped.load(Ordering::SeqCst));
}

#[test]
fn resume_denies_different_manager_or_different_logical_install_command() {
    let actor = identity();
    let original = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let deployment = plan().id();
    assert!(Request::resume(identity(), original, deployment, plan().project_id()).is_err());
    let changed = actor.with_idempotency_id(RequestId::new());
    assert!(Request::resume(changed, original, deployment, plan().project_id()).is_err());
}

#[test]
fn operation_entrypoints_without_original_ready_registry_admit_no_job() {
    let (service, control) = service(false);
    let actor = identity();
    let request = plan();
    let original = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    assert!(service.install(actor.clone(), request.clone()).is_err());
    assert!(
        service
            .resume(actor, original, request.id(), request.project_id())
            .is_err()
    );
    assert!(service.state().entries.is_empty());
    assert_eq!(control.calls.load(Ordering::SeqCst), 0);
    drop(service);
}

#[tokio::test]
async fn settled_denial_then_actual_second_command_panic_preserves_second_resume_cause() {
    let (service, mut control) = controlled_service(Some(1), Some(0));
    let request = plan();
    let first_actor = identity();
    let first_command =
        CommandIdentity::from_identity(&first_actor, DeploymentOperation::Install).unwrap();
    let denied = service
        .submit(Request::install(first_actor.clone(), request.clone()).unwrap())
        .unwrap();
    assert_eq!(
        control.entered.recv().await.unwrap().request,
        first_actor.request_id
    );
    let denial = denied.wait().await.unwrap_err();
    assert!(matches!(
        denial.as_ref(),
        Failure::Execution(crate::recipe_engine::RecipeApplicationError::Deployment(
            recipe_application::DeploymentError::AuthorizationDenied
        ))
    )); // Actual returned controlled denial; no ledger/provider effects exist in this seam.

    let second_actor = identity();
    let second_command =
        CommandIdentity::from_identity(&second_actor, DeploymentOperation::Install).unwrap();
    assert_ne!(first_command, second_command);
    let panicked = service
        .submit(Request::install(second_actor.clone(), request.clone()).unwrap())
        .unwrap();
    assert_eq!(
        control.entered.recv().await.unwrap().request,
        second_actor.request_id
    );
    assert!(panicked.wait().await.is_err());

    let wrong_resume = service
        .submit(
            Request::resume(
                first_actor,
                first_command,
                request.id(),
                plan().project_id(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(wrong_resume.wait().await.is_err());
    let replacement = service
        .submit(Request::install(identity(), request.clone()).unwrap())
        .unwrap();
    assert!(replacement.wait().await.is_err());
    assert_eq!(control.calls.load(Ordering::SeqCst), 2);

    let mut fresh = second_actor;
    fresh.request_id = RequestId::new();
    let resumed_request = fresh.request_id;
    let actual_resume = service
        .submit(Request::resume(fresh, second_command, request.id(), plan().project_id()).unwrap())
        .unwrap();
    let entered = control.entered.recv().await.unwrap();
    assert!(entered.resume);
    assert_eq!(entered.request, resumed_request);
    control.release.add_permits(1);
    assert!(actual_resume.wait().await.is_err());
    assert!(service.join_jobs().await.is_err());
    {
        let state = service.state();
        assert!(
            matches!(state.entries[&2].join_error.as_deref(), Some(Failure::Join(cause)) if cause.is_panic())
        );
        assert_eq!(state.entries[&5].request.original(), second_command);
        assert!(state.entries.values().all(|entry| entry.joined));
        drop(state);
    }
    drop(service);
    assert!(control.owner_dropped.load(Ordering::SeqCst));
}
