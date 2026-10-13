//! Read-only live snapshot DB contract. These trusted SQL fixtures do not
//! establish HTTP authentication, VM ownership, dispatch support or Invoke.
// Reuse the genuine cross-project authorization graph; its other helpers
// serve the original authz target and are not needed by every live case.
#[allow(dead_code)]
#[path = "../../authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Reuse the genuine cross-project authorization graph; its other helpers
// serve the original authz target and are not needed by every live case.
#[allow(dead_code)]
#[path = "../../authz-postgres/tests/postgres/support.rs"]
mod support;

mod live_snapshot_private {
    mod bindings;
    mod denials;
    mod fixture;
    mod issued;
    mod phases;
    mod sessions;
    mod updates;
    // Preserve the checked positive Git/update/mailbox producer correlations.
    // The original target uses additional helper entry points.
    #[allow(dead_code)]
    #[path = "../../../../runtime/volume/postgres/tests/run_profiles_private/producers.rs"]
    mod producers;
}
