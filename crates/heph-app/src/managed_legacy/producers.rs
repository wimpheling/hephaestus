//! Concrete compatible producer/completion adapters from the same worker pool.

use super::{LegacyVmPlacementScope, ManagedLegacyInputs};
use crate::UpdateRunCompletion;
use heph_run::{CompositeRunCompletionObserver, RunCompletionObserver};
use mailbox_dispatch::{MailboxDispatchStore, MailboxRunCompletion};
use mailbox_postgres::PostgresMailboxRepository;
use release_postgres::ReleaseService;
use run_domain::FreshRunExecutionProfile;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(super) struct Producers {
    // Keep the actual concrete profile-bound producers privately owned; no caller
    // can substitute an Arc with an invisible different producer configuration.
    _releases: Arc<ReleaseService>,
    _mailboxes: Arc<PostgresMailboxRepository>,
    pub(super) mailbox_store: Arc<dyn MailboxDispatchStore>,
    pub(super) completion: Arc<CompositeRunCompletionObserver>,
}

pub(super) fn build(inputs: &ManagedLegacyInputs, scope: &LegacyVmPlacementScope) -> Producers {
    let releases = Arc::new(
        ReleaseService::new(
            inputs.pools.worker.clone(),
            Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        )
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(scope.clone()),
    );
    let mailboxes = Arc::new(
        PostgresMailboxRepository::new(inputs.pools.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
            .with_legacy_vm_placement(scope.clone()),
    );
    let updates = Arc::new(UpdateRunCompletion {
        pool: inputs.pools.worker.clone(),
        releases: releases.clone(),
        admission_cursor: Mutex::new(None),
    });
    let mailbox_store: Arc<dyn MailboxDispatchStore> = mailboxes.clone();
    let completion = Arc::new(CompositeRunCompletionObserver::new(vec![
        updates as Arc<dyn RunCompletionObserver>,
        Arc::new(MailboxRunCompletion::new(mailbox_store.clone())),
    ]));
    Producers {
        _releases: releases,
        _mailboxes: mailboxes,
        mailbox_store,
        completion,
    }
}
