//! Controlled production lifecycle order; native/PG results are not claimed.
mod inventory_cases;
use super::{
    LifecycleActions, Operations, OrchestratorError, SupervisorReconciliationCause, inventory,
    reconcile,
};
use run_domain::{LegacyVmPlacementHold, LegacyVmPlacementHoldReason, LegacyVmPlacementInventory};
use runtime_types::RunId;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use vm_trait::VmProviderOwnerScope;

struct Controlled {
    scope: VmProviderOwnerScope,
    calls: Mutex<Vec<&'static str>>,
    later_held: bool,
    unresolved_after: bool,
    reconciled: Mutex<bool>,
    drained: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl Controlled {
    fn new() -> Self {
        Self {
            scope: VmProviderOwnerScope::new(RunId::new().to_string(), "host-1".into()).unwrap(),
            calls: Mutex::new(Vec::new()),
            later_held: false,
            unresolved_after: false,
            reconciled: Mutex::new(false),
            drained: Mutex::new(None),
        }
    }
    fn called(&self, call: &'static str) {
        self.calls.lock().unwrap().push(call);
    }
}
#[async_trait::async_trait]
impl LifecycleActions for Controlled {
    fn scope(&self) -> &VmProviderOwnerScope {
        &self.scope
    }
    fn validate(&self) -> Result<(), OrchestratorError> {
        self.called("validate");
        Ok(())
    }
    async fn invocation_inventory(&self) -> Result<BTreeSet<RunId>, OrchestratorError> {
        self.called("invocation-inventory");
        Ok(
            if self.unresolved_after && *self.reconciled.lock().unwrap() {
                BTreeSet::from([RunId::new()])
            } else {
                BTreeSet::new()
            },
        )
    }
    async fn legacy_inventory(&self) -> Result<LegacyVmPlacementInventory, OrchestratorError> {
        self.called("legacy-inventory");
        Ok(LegacyVmPlacementInventory {
            placements: Vec::new(),
            closed_runs: Vec::new(),
            held: if self.later_held {
                vec![LegacyVmPlacementHold {
                    run_id: RunId::new(),
                    reason: LegacyVmPlacementHoldReason::HistoricalUnproven,
                }]
            } else {
                Vec::new()
            },
        })
    }
    async fn close_provider(&self) -> Result<(), OrchestratorError> {
        self.called("close-provider");
        Ok(())
    }
    async fn drain_provider(&self) -> Result<(), OrchestratorError> {
        self.called("drain-provider");
        let signal = self.drained.lock().unwrap().take();
        if let Some(signal) = signal {
            signal.send(()).unwrap();
        }
        Ok(())
    }
    async fn reconcile_invocation(
        &self,
        _: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        self.called("invocation-effect");
        *self.reconciled.lock().unwrap() = true;
        Ok(())
    }
    async fn reconcile_legacy(
        &self,
        _: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        self.called("legacy-effect");
        Ok(())
    }
    async fn native_barrier(&self) -> Result<(), OrchestratorError> {
        self.called("native-barrier");
        Ok(())
    }
}

#[tokio::test]
async fn later_held_family_prevents_all_cleanup_and_physical_drain() {
    let mut actions = Controlled::new();
    actions.later_held = true;
    let operations = Operations::default();
    assert!(
        reconcile(&actions, &operations, SupervisorReconciliationCause::Stop)
            .await
            .is_err()
    );
    assert_eq!(
        *actions.calls.lock().unwrap(),
        [
            "validate",
            "close-provider",
            "invocation-inventory",
            "legacy-inventory"
        ]
    );
}

#[tokio::test]
async fn even_empty_qualified_snapshots_require_full_effect_postchecks_and_native_barrier() {
    let actions = Controlled::new();
    let operations = Operations::default();
    reconcile(
        &actions,
        &operations,
        SupervisorReconciliationCause::Restart,
    )
    .await
    .unwrap();
    assert_eq!(
        *actions.calls.lock().unwrap(),
        [
            "validate",
            "invocation-inventory",
            "legacy-inventory",
            "invocation-effect",
            "legacy-effect",
            "invocation-inventory",
            "legacy-inventory",
            "validate",
            "native-barrier"
        ]
    );
}

#[tokio::test]
async fn actionable_postinventory_cannot_become_native_or_ready_success() {
    let mut actions = Controlled::new();
    actions.unresolved_after = true;
    assert!(
        reconcile(
            &actions,
            &Operations::default(),
            SupervisorReconciliationCause::Restart
        )
        .await
        .is_err()
    );
    assert!(!actions.calls.lock().unwrap().contains(&"native-barrier"));
}

#[tokio::test]
async fn stop_joins_actual_registered_pipeline_before_reconciliation_effects() {
    use crate::managed_run_commands::{intake::Request, ownership};
    use heph_run::CancelRun;
    use runtime_types::CommandId;
    use tokio::sync::oneshot;
    let actions = Arc::new(Controlled::new());
    let operations = Arc::new(Operations::default());
    let (entered, running) = oneshot::channel();
    let (release, pipeline) = oneshot::channel();
    let actual = Arc::clone(&actions);
    let result = ownership::spawn(
        &operations,
        Request::Cancel(CancelRun {
            command_id: CommandId::new(),
            run_id: RunId::new(),
            reason: "real controlled command".into(),
        }),
        async move {
            entered.send(()).unwrap();
            pipeline.await.unwrap();
            actual.called("original-finalizer");
            Ok(())
        },
    )
    .unwrap();
    running.await.unwrap();
    drop(result);
    let (drained, physical_callback) = oneshot::channel();
    *actions.drained.lock().unwrap() = Some(drained);
    let worker = Arc::clone(&operations);
    let trace = Arc::clone(&actions);
    let stop_result = ownership::spawn(&operations, Request::Reconcile(SupervisorReconciliationCause::Stop), async move {
        reconcile(trace.as_ref(), &worker, SupervisorReconciliationCause::Stop).await
    }).unwrap();
    physical_callback.await.unwrap();
    drop(stop_result);
    assert!(!actions.calls.lock().unwrap().contains(&"invocation-effect"));
    let handle = operations.take_test_task(2);
    release.send(()).unwrap();
    handle.await.unwrap();
    let calls = actions.calls.lock().unwrap();
    let finalizer = calls
        .iter()
        .position(|call| *call == "original-finalizer")
        .unwrap();
    let effect = calls
        .iter()
        .position(|call| *call == "invocation-effect")
        .unwrap();
    assert!(finalizer < effect);
    drop(calls);
    drop(actions);
    drop(operations);
}
