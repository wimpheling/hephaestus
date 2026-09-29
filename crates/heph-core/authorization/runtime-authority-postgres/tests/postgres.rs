//! Real `PostgreSQL` proof for host-mediated gateway runtime sessions.

#[path = "postgres/fixtures.rs"]
mod fixtures;
#[path = "postgres/guest.rs"]
mod guest;
#[path = "postgres/host.rs"]
mod host;
#[path = "postgres/lease.rs"]
mod lease;
#[path = "postgres/race.rs"]
mod race;
#[path = "postgres/scenario.rs"]
mod scenario;
#[path = "postgres/support.rs"]
mod support;
