//! Ensures the browser E2E seed is rebuilt for every migration set.

use std::{env, fs, path::Path};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let migrations = Path::new(&manifest_dir).join("../../migrations");
    println!("cargo::rerun-if-changed={}", migrations.display());

    let mut entries = fs::read_dir(&migrations)
        .expect("workspace migrations directory exists")
        .map(|entry| {
            let entry = entry.expect("migration directory entry is readable");
            let bytes = fs::read(entry.path()).expect("migration file is readable");
            (entry.file_name(), bytes)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    // `sqlx::migrate!` embeds the migrations, but Cargo does not reliably
    // rebuild it when a migration file is newly added to a cached worktree.
    // Feed a deterministic manifest into rustc so the E2E seed binary always
    // embeds the exact schema it will bootstrap.
    let fingerprint = entries
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, entry| {
            entry
                .0
                .as_encoded_bytes()
                .iter()
                .chain(entry.1.iter())
                .fold(hash, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
                })
        });
    println!("cargo::rustc-env=HEPHAESTUS_MIGRATION_FINGERPRINT={fingerprint:016x}");
}
