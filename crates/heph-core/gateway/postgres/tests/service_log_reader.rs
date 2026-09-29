//! Real `PostgreSQL` coverage for authorized service-log epoch metadata reads.

#[path = "service_log_reader/authorization.rs"]
mod authorization;
#[path = "service_log_reader/authorized.rs"]
mod authorized;
#[path = "service_log_reader/pagination.rs"]
mod pagination;
#[path = "service_log_reader/project.rs"]
mod project;
#[path = "service_log_reader/retention.rs"]
mod retention;
#[path = "service_log_reader/snapshot.rs"]
mod snapshot;
#[path = "service_log_reader/support.rs"]
mod support;
