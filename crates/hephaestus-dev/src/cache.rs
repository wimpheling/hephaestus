use crate::{
    cli::CacheSelection,
    context::{DevContext, ELIXIR_IMAGE, FEDORA_IMAGE, NATS_IMAGE, POSTGRES_IMAGE},
    process::{Result, directory_size, remove_path, run},
};
use std::{path::Path, process::Command};

pub fn list(context: &DevContext) {
    print_cache("rust", &context.repository_root.join("target"));
    print_cache("elixir build", &context.repository_root.join("web/_build"));
    print_cache("elixir deps", &context.repository_root.join("web/deps"));
    print_cache(
        "browser node",
        &context.repository_root.join("e2e/playwright/node_modules"),
    );
    print_cache(
        "web node",
        &context.repository_root.join("web/assets/node_modules"),
    );
    println!("container images are stored in Podman's project-pinned image cache");
}

pub fn clean(context: &DevContext, selection: &CacheSelection) -> Result<()> {
    if selection.rust() {
        remove_path(&context.repository_root.join("target"))?;
    }
    if selection.elixir() {
        remove_path(&context.repository_root.join("web/_build"))?;
        remove_path(&context.repository_root.join("web/deps"))?;
    }
    if selection.node() {
        remove_path(&context.repository_root.join("e2e/playwright/node_modules"))?;
        remove_path(&context.repository_root.join("web/assets/node_modules"))?;
    }
    if selection.containers() {
        for image in [POSTGRES_IMAGE, NATS_IMAGE, ELIXIR_IMAGE, FEDORA_IMAGE] {
            let _ignored = run(Command::new("podman").args(["image", "rm", "--force", image]));
        }
    }
    println!("selected development caches cleaned");
    Ok(())
}

fn print_cache(label: &str, path: &Path) {
    let exists = path.exists();
    println!(
        "{label:16} {:>10}  {}",
        format_bytes(directory_size(path)),
        if exists {
            path.display().to_string()
        } else {
            format!("missing ({})", path.display())
        }
    );
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    const MIB: u64 = KIB * 1_024;
    const GIB: u64 = MIB * 1_024;
    if bytes >= GIB {
        format_scaled(bytes, GIB, "GiB")
    } else if bytes >= MIB {
        format_scaled(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_scaled(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

fn format_scaled(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = bytes % unit * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn formats_bounded_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
    }
}
