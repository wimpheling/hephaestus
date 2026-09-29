//! Generated protobuf checks.

use crate::{
    context::DevContext,
    process::{DevError, Result, run as run_process},
};
use std::process::Command;

use super::support::{cargo, phase};

pub(super) fn run(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    if !root.join("buf.yaml").is_file() {
        println!(
            "SKIP protobuf checks: RPC migration is gated; buf.yaml has not been introduced (Constraint 3)"
        );
        return Ok(());
    }

    let generation_script = root.join("scripts/check-generated.sh");
    if !generation_script.is_file() {
        return Err(DevError::Invalid(
            "scripts/check-generated.sh is required by RPC-GENERATED-FILES-CLEAN".to_owned(),
        ));
    }

    phase("deterministic generated bindings");
    run_process(Command::new(&generation_script).current_dir(root))?;

    let buf = root.join(".local/protobuf/bin/buf");
    phase("protobuf format");
    run_process(
        Command::new(&buf)
            .args(["format", "--diff", "--exit-code"])
            .current_dir(root),
    )?;
    phase("protobuf lint");
    run_process(Command::new(&buf).arg("lint").current_dir(root))?;

    phase("protobuf descriptor policy");
    cargo(
        root,
        &[
            "test",
            "-p",
            "rpc-proto",
            "--test",
            "descriptor_policy",
            "--all-features",
        ],
    )?;

    phase("protobuf compatibility");
    run_process(Command::new(root.join("scripts/check-protobuf-breaking.sh")).current_dir(root))?;
    Ok(())
}
