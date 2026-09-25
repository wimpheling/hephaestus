use std::time::Duration;

use crate::policy::{bounded_reason, isolated_request, validate_output};
use crate::{
    ClaimedProductionJob, OciBuildEngine, OciImageProductionJobStore, OciImageProductionOutput,
    OciWorkerError, RepositoryOciImageProvenance, SourceCheckoutProvider,
};

/// Isolated OCI production worker.
pub struct OciImageProductionWorker<S, C, E> {
    store: S,
    checkout: C,
    engine: E,
    worker_name: String,
    materialization_worker_name: String,
    lease: Duration,
}

impl<S, C, E> OciImageProductionWorker<S, C, E>
where
    S: OciImageProductionJobStore,
    C: SourceCheckoutProvider,
    E: OciBuildEngine,
{
    /// Creates a worker with a stable operator-visible identity.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty identity or zero lease.
    pub fn new(
        store: S,
        checkout: C,
        engine: E,
        worker_name: String,
        materialization_worker_name: String,
        lease: Duration,
    ) -> Result<Self, OciWorkerError> {
        if worker_name.trim().is_empty()
            || worker_name.len() > 200
            || materialization_worker_name.trim().is_empty()
            || materialization_worker_name.len() > 200
            || lease.is_zero()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            store,
            checkout,
            engine,
            worker_name,
            materialization_worker_name,
            lease,
        })
    }

    /// Processes at most one durable job. Redelivery is harmless because the
    /// store's claim and terminal transitions are compare-and-swap operations.
    ///
    /// # Errors
    ///
    /// Returns a checkout, Dockerfile policy, engine, or durable-store error.
    pub async fn run_once(&self) -> Result<bool, OciWorkerError> {
        let Some(job) = self
            .store
            .claim_production(&self.worker_name, self.lease)
            .await
            .map_err(OciWorkerError::Store)?
        else {
            return Ok(false);
        };
        let result = self.prepare(&job).await;
        match result {
            Ok((output, provenance)) => self
                .store
                .complete_production(
                    job.id,
                    &self.materialization_worker_name,
                    &output,
                    provenance,
                )
                .await
                .map_err(OciWorkerError::Store)?,
            Err(error) => self
                .store
                .fail_production(job.id, &bounded_reason(&error))
                .await
                .map_err(OciWorkerError::Store)?,
        }
        Ok(true)
    }

    async fn prepare(
        &self,
        job: &ClaimedProductionJob,
    ) -> Result<(OciImageProductionOutput, RepositoryOciImageProvenance), OciWorkerError> {
        let source = self.checkout.checkout(job).await?;
        let result = async {
            let request = isolated_request(job, &source)?;
            let output = self.engine.build(request).await?;
            validate_output(job, &output)?;
            let provenance = RepositoryOciImageProvenance {
                source_revision: job.source_revision.clone(),
                context_digest: job.context_digest.clone(),
                attestation_reference: output.attestation_reference.clone(),
                sbom_reference: output.sbom_reference.clone(),
            };
            provenance
                .validate()
                .map_err(|_| OciWorkerError::InvalidOutput)?;
            Ok((output, provenance))
        }
        .await;
        self.checkout.cleanup(&source).await?;
        result
    }
}
