# One owned volume creation

`PreparedOwnedVolumeCreator` creates or resumes one original owned volume from a
prepared installation. It requires an existing configured disk root owner and
real actor/worker repositories. It initializes no provider root, chooses no VM,
and runs no other resource effects.

Whole preparation and the exact active original Create claim are checked before
registration. The per-effect actor wrapper repeats those checks, current source
and configuration validation, and full creation mapping comparison whenever the
Local provider requests fresh admission before namespace, allocation or first
format. Recovery keeps the original provisioning operation and generation;
unknown progress cannot allocate another operation or format again.

Ready before protected completion requires fresh filesystem verification and
non-clone journal custody. The service moves custody into a detached owning
completion task before awaiting the recorder or actor transaction. That owner
keeps custody through fresh preparation, recording, commit, and any ambiguity
bookkeeping. Cancelling the caller does not cancel an in-flight completion.
Completed protected replay returns progress before discovery or fsck, subject to
current original Install authorization. No runtime lease is acquired here.

The dedicated durable first-birth guard must already be installed: every runtime
lease stays excluded until this original Create has immutable successful
completion. Metadata Ready alone cannot open admission. The service grants no
launch, source delegation, mount grant or runtime capability.

Errors retain the primary failure and separately expose denied ambiguity
recording. An unknown join/commit is held, not NotApplied or physical absence.
Ready source withdrawal and Remove require future manager-qualified Ready
retention; this service never relabels Ready as partial or borrows Install
identity to remove it. Ordinary attached Ready reconciliation is unchanged.
