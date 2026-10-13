//! One retained drain job; cancelling its join never restarts consuming shutdown.
//!
//! The caller must retain the job on error/cancel for explicit C recovery. Whole
//! job/runtime/process loss is unsupported: Tokio may detach a dropped handle,
//! and no dropped owner, task or future certifies stopped or performs cleanup.

use super::super::{
    publication::{
        PublicationCheckpoint, StoppedPublicationBackend, require_workspaces_absent,
        verify_pair_state,
    },
    publication_provider::{
        ObservedWorkerExit, ProvisionOutcome, ProvisioningSnapshot, PublicationProvider,
    },
};
use super::{BuildError, PublicationBackend, PublicationPins, PublishedVariant, unavailable};
use hephaestus_app::AppError;
use release_service::InstanceExecutionConfiguration;
use sqlx::PgPool;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::{sync::Mutex, task::JoinHandle};
use vm_libkrun::LibkrunProvider;
use vm_trait::{VmId, VmProvider};
use volume_local::VolumeRootOwner;

type Shutdown = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + 'static>>;

enum Phase {
    Retained,
    Shutdown,
    Closed,
    Cleaning,
    Physical,
    Verified,
    Taken,
    Held,
}

struct DrainOwner {
    backend: PublicationBackend,
    shutdown: Option<Shutdown>,
    checkpoint: Option<PublicationCheckpoint>,
    cleanup_index: usize,
    phase: Phase,
    failure: Option<String>,
    proof: Option<DrainProof>,
}

/// A private custody handle; retain it until the same job joins or is archived.
#[must_use]
pub(in super::super) struct PublicationDrainJob {
    retained: Arc<Mutex<DrainOwner>>,
    task: Option<JoinHandle<()>>,
    joined: bool,
    join_failure: Option<String>,
}

impl PublicationDrainJob {
    pub(super) fn start(backend: PublicationBackend) -> Self {
        let runtime = tokio::runtime::Handle::try_current();
        let failure = runtime.as_ref().err().map(ToString::to_string);
        let retained = Arc::new(Mutex::new(DrainOwner {
            backend,
            shutdown: None,
            checkpoint: None,
            cleanup_index: 0,
            phase: if failure.is_some() {
                Phase::Held
            } else {
                Phase::Retained
            },
            failure,
            proof: None,
        }));
        let task = runtime.ok().map(|runtime| {
            let owner = retained.clone();
            runtime.spawn(async move {
                let mut owner = owner.lock().await;
                if let Err(error) = drain_owned(&mut owner).await {
                    owner.failure = Some(error.to_string());
                    owner.phase = Phase::Held;
                }
            })
        });
        Self {
            retained,
            task,
            joined: false,
            join_failure: None,
        }
    }

    /// Rejoins this single handle; cancellation leaves its task/future retained.
    pub(in super::super) async fn join(&mut self) -> Result<(), BuildError> {
        if !self.joined {
            if let Some(task) = self.task.as_mut() {
                self.join_failure = task.await.err().map(|error| error.to_string());
            }
            self.joined = true;
            self.task = None;
        }
        let mut owner = self.retained.lock().await;
        if let Some(error) = &self.join_failure {
            owner.failure = Some(error.clone());
            owner.phase = Phase::Held;
        }
        let failure = owner.failure.clone();
        let completed = matches!(owner.phase, Phase::Verified | Phase::Taken);
        drop(owner);
        if let Some(error) = failure {
            return Err(std::io::Error::other(error).into());
        }
        if !completed {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Consumes the proof once, after the protected task actually joined.
    pub(in super::super) fn take_stopped_pair(
        &self,
    ) -> Result<[StoppedPublicationBackend; 2], BuildError> {
        if !self.joined || self.join_failure.is_some() {
            return Err(unavailable());
        }
        let mut owner = self.retained.try_lock().map_err(|_| unavailable())?;
        if owner.failure.is_some() || !matches!(owner.phase, Phase::Verified) {
            return Err(unavailable());
        }
        let proof = owner.proof.take().ok_or_else(unavailable)?;
        owner.phase = Phase::Taken;
        drop(owner);
        Ok(StoppedPublicationBackend::from_drain(proof))
    }
}

/// Constructed only after real joins, scoped cleanup and all pinned checks succeed.
pub(in super::super) struct DrainProof {
    pair: [Arc<PublishedVariant>; 2],
    pins: [PublicationPins; 2],
    handoff: Arc<InvocationBackendCapability>,
    workers: Vec<ObservedWorkerExit>,
}

// An opaque post-proof allocation identity. It intentionally offers no generic
// VmProvider/raw-Arc getter. Real Invocation admission and the shared operation
// guard across all later controllers require a separate managed composition.
struct InvocationBackendCapability {
    actual: Arc<LibkrunProvider>,
    gate: Arc<PublicationProvider>,
    owner: Arc<VolumeRootOwner>,
    configuration: InstanceExecutionConfiguration,
    admin: PgPool,
}

impl DrainProof {
    pub(in super::super) const fn publication(&self, index: usize) -> &Arc<PublishedVariant> {
        &self.pair[index]
    }
    pub(in super::super) fn same_handoff(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.handoff, &other.handoff)
    }
    pub(in super::super) fn validate_handoff(&self, index: usize) -> Result<(), BuildError> {
        let p = &self.pair[index];
        self.pins[index].verify()?;
        if self.workers.is_empty() {
            return Err(unavailable());
        }
        self.handoff.validate(p)
    }
}

impl InvocationBackendCapability {
    fn validate(&self, p: &PublishedVariant) -> Result<(), BuildError> {
        self.owner.validate()?;
        let gate: Arc<dyn VmProvider> = self.gate.clone();
        let actual: Arc<dyn VmProvider> = self.actual.clone();
        if !self.gate.retains_provider(&actual)
            || !Arc::ptr_eq(&gate, &p.provider)
            || !Arc::ptr_eq(&self.owner, &p.owner)
            || self.actual.owner_scope()? != *self.configuration.provider_scope()
            || p.configuration.canonical_json() != self.configuration.canonical_json()
            || !std::ptr::eq(self.admin.options(), p.pool.options())
            || !Arc::ptr_eq(&self.admin.connect_options(), &p.pool.connect_options())
            || self.admin.is_closed()
        {
            return Err(unavailable());
        }
        self.gate.provisioning_snapshot()?.require_all_returned()?;
        Ok(())
    }
}

async fn drain_owned(owner: &mut DrainOwner) -> Result<(), BuildError> {
    let pair = owner.backend.pair()?;
    owner.backend.validate_pair(&pair)?;
    let pins = retained_pins(&owner.backend)?;
    owner.checkpoint = Some(verify_pair_state(&pair, &pins).await?);
    // Store the consuming future in retained owner BEFORE its first poll. No
    // cancelled join takes/drops it, and neither errors nor panics are retried.
    owner.shutdown = Some(Box::pin(
        owner
            .backend
            .running
            .take()
            .ok_or_else(unavailable)?
            .shutdown(),
    ));
    owner.phase = Phase::Shutdown;
    owner
        .shutdown
        .as_mut()
        .ok_or_else(unavailable)?
        .as_mut()
        .await?;
    let app_pools = owner.backend.app_pools.as_ref().ok_or_else(unavailable)?;
    let admin = owner.backend.pool()?;
    if app_pools
        .iter()
        .any(|pool| !pool.is_closed() || std::ptr::eq(pool.options(), admin.options()))
    {
        return Err(unavailable());
    }
    let gate = owner.backend.provider()?.clone();
    gate.close_provisioning()?;
    owner.phase = Phase::Closed;
    let snapshot = gate.provisioning_snapshot()?;
    let checkpoint = owner.checkpoint.as_ref().ok_or_else(unavailable)?;
    let ids = require_expected_attempts(&snapshot, checkpoint)?;
    let after = verify_pair_state(&pair, &retained_pins(&owner.backend)?).await?;
    if !checkpoint.matches(&after)? {
        return Err(unavailable());
    }
    owner.phase = Phase::Cleaning;
    for (index, id) in ids.iter().enumerate() {
        owner.cleanup_index = index;
        gate.cleanup_orphan_scoped(gate.retained_scope(), id)
            .await?;
        owner.cleanup_index = index + 1;
    }
    owner.phase = Phase::Physical;
    let workers = gate.verify_physical_after_cleanup()?;
    if workers.len() != snapshot.attempts().len() {
        return Err(unavailable());
    }
    owner.backend.validate_pair(&pair)?;
    let final_state = verify_pair_state(&pair, &retained_pins(&owner.backend)?).await?;
    if !checkpoint.matches(&final_state)? {
        return Err(unavailable());
    }
    require_workspaces_absent(&pair, checkpoint)?;
    owner.proof = Some({
        let handoff = Arc::new(InvocationBackendCapability {
            actual: owner
                .backend
                .actual_provider
                .as_ref()
                .ok_or_else(unavailable)?
                .clone(),
            gate,
            owner: owner
                .backend
                .owner
                .as_ref()
                .ok_or_else(unavailable)?
                .clone(),
            configuration: owner
                .backend
                .configuration
                .as_ref()
                .ok_or_else(unavailable)?
                .clone(),
            admin: owner.backend.pool()?.clone(),
        });
        handoff.validate(&pair[0])?;
        handoff.validate(&pair[1])?;
        let pins = [
            owner.backend.pins[0].take().ok_or_else(unavailable)?,
            owner.backend.pins[1].take().ok_or_else(unavailable)?,
        ];
        DrainProof {
            pair,
            pins,
            handoff,
            workers,
        }
    });
    owner.phase = Phase::Verified;
    Ok(())
}

fn retained_pins(backend: &PublicationBackend) -> Result<[&PublicationPins; 2], BuildError> {
    Ok([
        backend.pins[0].as_ref().ok_or_else(unavailable)?,
        backend.pins[1].as_ref().ok_or_else(unavailable)?,
    ])
}

fn require_expected_attempts(
    snapshot: &ProvisioningSnapshot,
    checkpoint: &PublicationCheckpoint,
) -> Result<[VmId; 2], BuildError> {
    snapshot.require_all_returned()?;
    let mut seen = [false; 2];
    for attempt in snapshot.attempts() {
        if attempt.outcome() != ProvisionOutcome::ReturnedHandle {
            return Err(unavailable());
        }
        let index = checkpoint
            .expected
            .iter()
            .position(|id| id == attempt.id())
            .ok_or_else(unavailable)?;
        seen[index] = true;
    }
    if !seen.into_iter().all(|seen| seen) || checkpoint.expected[0] == checkpoint.expected[1] {
        return Err(unavailable());
    }
    Ok(checkpoint.expected.clone())
}
