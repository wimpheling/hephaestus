//! Real `PostgreSQL` coverage for persistent gateway service targets and logs.

mod service_targets {
    mod instance_inventory;
    mod instance_lookup;
    mod log_append;
    mod maintenance_eligibility;
    mod maintenance_epoch;
    mod maintenance_expiry;
    mod maintenance_page;
    mod maintenance_pressure;
    mod maintenance_project_pressure;
    mod maintenance_serialization;
    mod maintenance_watermark;
    mod support;
    mod target_boundaries;
    mod uuid_pages;
}
