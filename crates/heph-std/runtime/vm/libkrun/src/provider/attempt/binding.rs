//! Producer-owned phase/binding: no missing entry implies physical absence.
use crate::provider::common::{Arc, LibkrunInstance, PendingLaunch};
use std::sync::{Mutex, MutexGuard};
use vm_trait::{VmError, VmInstance, VmSpec};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Fresh,
    Preparing,
    Reserving,
    NotAdmitted,
    Admitted,
    Cleaned,
    Unknown,
}
struct Data {
    phase: Phase,
    closed: bool,
    spec: Option<VmSpec>,
    pending: Option<Arc<PendingLaunch>>,
    instance: Option<Arc<dyn VmInstance>>,
    outcome: Option<Result<Arc<dyn VmInstance>, Arc<VmError>>>,
    errors: Vec<Arc<VmError>>,
    launch_jobs: Vec<Arc<tokio::sync::Mutex<super::jobs::LaunchJob>>>,
    #[cfg(test)]
    close_on_capture: bool,
}
#[derive(Clone)]
pub(in super::super) struct Binding(Arc<Mutex<Data>>);
pub(super) enum Start {
    Spec(Box<VmSpec>),
    Cached(Result<Arc<dyn VmInstance>, Arc<VmError>>),
}
pub(super) enum Cleanup {
    NoEffects,
    Pending {
        pending: Arc<PendingLaunch>,
        instance: Option<Arc<dyn VmInstance>>,
    },
}
impl Binding {
    pub(in super::super) fn new(spec: VmSpec) -> Self {
        Self(Arc::new(Mutex::new(Data {
            phase: Phase::Fresh,
            closed: false,
            spec: Some(spec),
            pending: None,
            instance: None,
            outcome: None,
            errors: Vec::new(),
            launch_jobs: Vec::new(),
            #[cfg(test)]
            close_on_capture: false,
        })))
    }
    fn data(&self) -> MutexGuard<'_, Data> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(super) fn start(&self) -> Result<Start, VmError> {
        let mut data = self.data();
        if data.closed {
            return Err(VmError::Destroyed);
        }
        if let Some(outcome) = &data.outcome {
            return Ok(Start::Cached(outcome.clone()));
        }
        if data.phase != Phase::Fresh {
            return Err(VmError::InvalidState(
                "original provision outcome remains unknown",
            ));
        }
        let spec = data
            .spec
            .take()
            .ok_or(VmError::InvalidState("original spec missing"))?;
        data.phase = Phase::Preparing;
        drop(data);
        Ok(Start::Spec(Box::new(spec)))
    }
    // Called under original ids/admission lock, before the native admit/spawn.
    pub(in super::super) fn reserve(&self, pending: Arc<PendingLaunch>) -> Result<(), VmError> {
        let mut data = self.data();
        if data.phase != Phase::Preparing || data.pending.is_some() {
            data.phase = Phase::Unknown;
            return Err(VmError::InvalidState("original launch binding changed"));
        }
        if data.closed {
            return Err(VmError::Destroyed);
        }
        data.pending = Some(pending);
        data.phase = Phase::Reserving;
        drop(data);
        Ok(())
    }
    pub(in super::super) fn not_admitted(&self) {
        self.data().phase = Phase::NotAdmitted;
    }
    // Positive native producer admission, no await before owner task spawn.
    pub(in super::super) fn admitted(&self) {
        let mut data = self.data();
        data.phase = Phase::Admitted;
        if data.closed
            && let Some(pending) = &data.pending
        {
            pending.request_drain();
        }
    }
    // Pin SAME original instance before native handoff acknowledgement/retirement.
    pub(in super::super) fn instance(&self, instance: Arc<LibkrunInstance>) -> Result<(), VmError> {
        let mut data = self.data();
        let instance: Arc<dyn VmInstance> = instance;
        if data.phase != Phase::Admitted
            || data
                .pending
                .as_ref()
                .is_none_or(|pending| &pending.id != instance.id())
            || data
                .instance
                .as_ref()
                .is_some_and(|prior| !Arc::ptr_eq(prior, &instance))
        {
            data.phase = Phase::Unknown;
            return Err(VmError::InvalidState("original instance binding changed"));
        }
        data.instance = Some(instance);
        #[cfg(test)]
        if data.close_on_capture {
            data.closed = true;
            if let Some(pending) = &data.pending {
                pending.request_drain();
            }
        }
        if data.closed {
            return Err(VmError::Destroyed);
        }
        drop(data);
        Ok(())
    }
    pub(in super::super) fn close(&self) {
        let mut data = self.data();
        data.closed = true;
        if let Some(pending) = &data.pending {
            pending.request_drain();
        }
        drop(data);
    }
    pub(super) fn finish(
        &self,
        result: Result<Arc<dyn VmInstance>, VmError>,
    ) -> Result<Arc<dyn VmInstance>, VmError> {
        let mut data = self.data();
        let result = result.map_err(Arc::new);
        if result.is_err() && data.phase == Phase::Preparing {
            // Producer returned BEFORE reservation: all preceding operations are
            // readonly validation, with no spawn or allocation. Panic has no finish.
            data.phase = Phase::NotAdmitted;
        }
        if let Err(error) = &result {
            data.errors.push(error.clone());
        }
        data.outcome = Some(result.clone());
        match result {
            Err(original) => Err(super::retained_error(original)),
            Ok(_) if data.closed => Err(VmError::Destroyed),
            Ok(instance) => Ok(instance),
        }
    }
    pub(super) fn cleanup_target(&self) -> Result<Cleanup, VmError> {
        let data = self.data();
        match data.phase {
            Phase::Fresh
                if data.spec.is_some()
                    && data.pending.is_none()
                    && data.instance.is_none()
                    && data.outcome.is_none() =>
            {
                Ok(Cleanup::NoEffects)
            }
            Phase::NotAdmitted if data.instance.is_none() => Ok(Cleanup::NoEffects),
            Phase::Cleaned if data.closed => Ok(Cleanup::NoEffects),
            Phase::Admitted => {
                let pending = data
                    .pending
                    .clone()
                    .ok_or(VmError::InvalidState("admitted original launch missing"))?;
                // A captured pre-ack instance is NOT a completed handoff. The
                // job first drains/joins this exact owner and checks final state.
                Ok(Cleanup::Pending {
                    pending,
                    instance: data.instance.clone(),
                })
            }
            Phase::Fresh
            | Phase::NotAdmitted
            | Phase::Cleaned
            | Phase::Preparing
            | Phase::Reserving
            | Phase::Unknown => Err(VmError::InvalidState(
                "original admission outcome remains unknown",
            )),
        }
    }
    // Called immediately after native spawn, with no await or ownership gap.
    pub(in super::super) fn retain_launch_owner(
        &self,
        task: tokio::task::JoinHandle<()>,
    ) -> Result<(), VmError> {
        let mut data = self.data();
        let valid =
            data.phase == Phase::Admitted && data.pending.is_some() && data.launch_jobs.is_empty();
        // Preserve even an unexpected extra actual owner; never detach/drop it
        // as a substitute for qualifying the original binding.
        data.launch_jobs.push(Arc::new(tokio::sync::Mutex::new(
            super::jobs::LaunchJob::new(task),
        )));
        if !valid {
            data.phase = Phase::Unknown;
            drop(data);
            return Err(VmError::InvalidState(
                "original launch owner binding changed",
            ));
        }
        drop(data);
        Ok(())
    }
    pub(super) fn launch_owner(
        &self,
    ) -> Result<Arc<tokio::sync::Mutex<super::jobs::LaunchJob>>, VmError> {
        let data = self.data();
        match data.launch_jobs.as_slice() {
            [job] => Ok(job.clone()),
            _ => Err(VmError::InvalidState(
                "original launch owner missing or ambiguous",
            )),
        }
    }
    #[cfg(test)]
    pub(super) fn close_on_capture(&self) {
        self.data().close_on_capture = true;
    }
    pub(super) fn cleaned(&self) {
        self.data().phase = Phase::Cleaned;
    }
    pub(super) fn error(&self, error: VmError) -> VmError {
        let mut data = self.data();
        data.errors.push(Arc::new(error));
        let Some(original) = data.errors.last().cloned() else {
            return VmError::InvalidState("original cleanup error missing");
        };
        drop(data);
        super::retained_error(original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_or_unknown_original_binding_cannot_certify_cleanup() {
        for phase in [
            Phase::Fresh,
            Phase::Preparing,
            Phase::Reserving,
            Phase::Admitted,
            Phase::Unknown,
        ] {
            let binding = Binding(Arc::new(Mutex::new(Data {
                phase,
                closed: false,
                spec: None,
                pending: None,
                instance: None,
                outcome: None,
                errors: Vec::new(),
                launch_jobs: Vec::new(),
                close_on_capture: false,
            })));
            binding.close();
            assert!(binding.cleanup_target().is_err());
            assert!(matches!(binding.start(), Err(VmError::Destroyed)));
            drop(binding);
        }
    }
}
