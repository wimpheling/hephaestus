//! Genuine ordinary public139 owner history, without VM or backing IO.

#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/fixture.rs"]
#[allow(dead_code)] // Reuse the genuine ordinary fixture; its routing checks are a separate target.
mod fixture;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/git.rs"]
mod git;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/parents.rs"]
mod parents;
#[path = "public139_vm_owner_families/schema.rs"]
mod schema;

#[path = "public139_vm_owner_families/invocation.rs"]
mod invocation;

#[path = "public139_vm_owner_families/mailbox.rs"]
mod mailbox;
