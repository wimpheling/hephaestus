// Exact original source labels resolve to byte-qualified committed assets.
pub fn source_path(name: &str) -> std::path::PathBuf {
    let relative = match name {
        "0115_qualified_instance_invocation.sql" => {
            "migrations/0120_qualified_instance_invocation.sql"
        }
        "0117_qualified_invocation_consumption.nowait.sql" => {
            "migrations/0122_qualified_invocation_consumption.sql"
        }
        "0119_qualified_invocation_authority.sql" => {
            "migrations/0124_qualified_invocation_authority.sql"
        }
        "0119_qualified_invocation_empty_secret.sql" => {
            "migrations/0125_qualified_invocation_empty_secret.sql"
        }
        "0119_qualified_invocation_leases.sql" => "migrations/0123_qualified_invocation_leases.sql",
        "0119_qualified_invocation_pre_acquisition.sql" => {
            "migrations/0126_qualified_invocation_pre_acquisition.sql"
        }
        _ => panic!("unknown private SQL source label"),
    };
    let levels = match env!("CARGO_PKG_NAME") {
        "runtime-authority-postgres" => 4,
        "hephaestus-app" => 2,
        _ => panic!("unexpected private fixture manifest"),
    };
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(levels)
        .expect("private fixture workspace root")
        .join(relative)
}
