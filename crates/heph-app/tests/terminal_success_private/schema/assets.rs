// Exact original source labels resolve to byte-qualified committed assets.
pub fn source_path(name: &str) -> std::path::PathBuf {
    let relative = match name {
        "0108_legacy_scalar_compatibility.sql" => "migrations/0110_legacy_scalar_compatibility.sql",
        "0108_legacy_vm_placement.sql" => "migrations/0112_legacy_vm_placement.sql",
        "0108_run_cleanup_fragment.profile.sql" => "migrations/0109_run_cleanup.sql",
        "0108_run_profile_lifecycle.sql" => "migrations/0111_run_profile_lifecycle.sql",
        "0108_run_volume_leases.profile.sql" => "migrations/0108_run_volume_leases.sql",
        "0110_owned_volume_registration.sql" => "migrations/0114_owned_volume_registration.sql",
        "0111_owned_volume_backing.sql" => "migrations/0115_owned_volume_backing.sql",
        "0112_owned_partial_retention.atomic.sql" => "migrations/0116_owned_partial_retention.sql",
        "0113_deployment_execution_preparation.sql" => {
            "migrations/0117_deployment_execution_preparation.sql"
        }
        "0113_owned_first_ready_lease_exclusion.sql" => {
            "migrations/0118_owned_first_ready_lease_exclusion.sql"
        }
        "0114_qualified_instance_activation.corrected.sql" => {
            "migrations/0119_qualified_instance_activation.sql"
        }
        "0115_qualified_instance_invocation.sql" => {
            "migrations/0120_qualified_instance_invocation.sql"
        }
        "0116_owned_ready_retention.sql" => "migrations/0121_owned_ready_retention.sql",
        "0117_qualified_invocation_consumption.nowait.sql" => {
            "migrations/0122_qualified_invocation_consumption.sql"
        }
        "0118_existing_instance_closure.sql" => "migrations/0130_existing_instance_closure.sql",
        "0118_prepared_instance_closure.sql" => "migrations/0128_prepared_instance_closure.sql",
        "0118_prepared_instance_safety.sql" => "migrations/0127_prepared_instance_safety.sql",
        "0118_qualified_remove_resume.sql" => "migrations/0129_qualified_remove_resume.sql",
        "0118_run_cleanup_wake_qualification.sql" => {
            "migrations/0131_run_cleanup_wake_qualification.sql"
        }
        "0119_qualified_invocation_authority.sql" => {
            "migrations/0124_qualified_invocation_authority.sql"
        }
        "0119_qualified_invocation_empty_secret.sql" => {
            "migrations/0125_qualified_invocation_empty_secret.sql"
        }
        "0119_qualified_invocation_leases.sql" => "migrations/0123_qualified_invocation_leases.sql",
        "0119_qualified_invocation_remove_resume.sql" => {
            "migrations/0132_qualified_invocation_remove_resume.sql"
        }
        "0119_qualified_invocation_terminal_success.empty_secret_cleanup.sql" => {
            "migrations/0137_qualified_invocation_terminal_success.sql"
        }
        "0119_queued_invocation_cancel_delivery.sql" => {
            "migrations/0134_queued_invocation_cancel_delivery.sql"
        }
        "0119_queued_invocation_cleanup.sql" => "migrations/0133_queued_invocation_cleanup.sql",
        "0119_queued_invocation_cleanup_context.sql" => {
            "migrations/0135_queued_invocation_cleanup_context.sql"
        }
        "0119_queued_invocation_cleanup_transaction.sql" => {
            "migrations/0136_queued_invocation_cleanup_transaction.sql"
        }
        "heph-recipe-0109_external_observation_retirement.sql" => {
            "migrations/0113_external_observation_retirement.sql"
        }
        _ => panic!("unknown private SQL source label"),
    };
    let levels = match env!("CARGO_PKG_NAME") {
        "hephaestus-app" => 2,
        _ => panic!("unexpected private fixture manifest"),
    };
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(levels)
        .expect("private fixture workspace root")
        .join(relative)
}
