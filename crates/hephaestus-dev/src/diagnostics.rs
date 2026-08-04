use crate::{
    cli::{LogArgs, LogComponent},
    context::DevContext,
    process::{DevError, Result, command_exists, output, run, run_quiet},
    state, zot,
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

const REQUIRED_COMMANDS: [&str; 16] = [
    "awk",
    "cargo",
    "curl",
    "git",
    "install",
    "ldconfig",
    "mkfs.ext4",
    "musl-gcc",
    "node",
    "npm",
    "podman",
    "rustup",
    "tar",
    "unshare",
    "kill",
    "tail",
];

pub fn doctor(_context: &DevContext) -> Result<()> {
    let mut failures = Vec::new();
    for command in REQUIRED_COMMANDS {
        report(
            &mut failures,
            format!("command {command}"),
            command_exists(command),
        );
    }
    report(
        &mut failures,
        "x86_64 host".into(),
        std::env::consts::ARCH == "x86_64",
    );
    report(
        &mut failures,
        "/dev/kvm readable and writable".into(),
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok(),
    );
    report(
        &mut failures,
        "/usr/bin/passt executable".into(),
        fs::metadata("/usr/bin/passt")
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0),
    );
    let loader_cache = output("ldconfig", &["-p"]).unwrap_or_default();
    report(
        &mut failures,
        "libkrun.so.1 available".into(),
        loader_cache.contains("libkrun.so.1"),
    );
    report(
        &mut failures,
        "libkrunfw.so.5 available".into(),
        loader_cache.contains("libkrunfw.so.5"),
    );
    report(
        &mut failures,
        "UID/GID 10001 user namespace".into(),
        run_quiet(
            "unshare",
            &["--map-user", "10001", "--map-group", "10001", "true"],
        )
        .unwrap_or(false),
    );
    if failures.is_empty() {
        println!("development host is ready");
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "{} prerequisite checks failed: {}",
            failures.len(),
            failures.join(", ")
        )))
    }
}

pub fn status(context: &DevContext) -> Result<()> {
    let supervisor = supervisor_status(context)?;
    println!("supervisor       {supervisor}");
    print_service(
        "web",
        run_quiet("curl", &["--fail", "--silent", "http://127.0.0.1:4000/"])?,
    );
    print_service(
        "daemon",
        run_quiet(
            "curl",
            &["--fail", "--silent", "http://127.0.0.1:8080/healthz"],
        )?,
    );
    print_service(
        "oidc",
        run_quiet(
            "curl",
            &[
                "--fail",
                "--silent",
                "http://127.0.0.1:5556/.well-known/openid-configuration",
            ],
        )?,
    );
    print_service(
        "zot",
        zot::running(context)? && zot::challenge_ready(context)?,
    );
    for container in [
        context.postgres_container(),
        context.nats_container(),
        context.web_container(),
        context.zot_container(),
    ] {
        let running = run_quiet(
            "podman",
            &["inspect", "--format", "{{.State.Running}}", &container],
        )?;
        println!(
            "{container:16} {}",
            if running { "running" } else { "stopped" }
        );
    }
    println!();
    state::list(context)
}

pub fn logs(context: &DevContext, arguments: &LogArgs) -> Result<()> {
    if let Some(component) = arguments.component {
        return show_log(context, component, arguments.follow);
    }
    for component in [
        LogComponent::Daemon,
        LogComponent::Oidc,
        LogComponent::Web,
        LogComponent::Zot,
    ] {
        println!("== {} ==", component_name(component));
        show_log(context, component, false)?;
    }
    Ok(())
}

fn show_log(context: &DevContext, component: LogComponent, follow: bool) -> Result<()> {
    if matches!(component, LogComponent::Zot) {
        return zot::show_logs(context, follow);
    }
    let filename = match component {
        LogComponent::Web => "web.log",
        LogComponent::Daemon => "daemon.log",
        LogComponent::Oidc => "oidc.log",
        LogComponent::Zot => unreachable!("Zot logs are provided by Podman"),
    };
    let path = context.logs().join(filename);
    if !path.exists() {
        println!("missing ({})", path.display());
        return Ok(());
    }
    let mut command = Command::new("tail");
    if follow {
        command.args(["-f", "-n", "200"]);
    } else {
        command.args(["-n", "200"]);
    }
    command.arg(path);
    run(&mut command)
}

fn supervisor_status(context: &DevContext) -> Result<String> {
    let Ok(pid) = fs::read_to_string(context.supervisor_pid_file()) else {
        return Ok("stopped".into());
    };
    let pid = pid.trim();
    if !pid.is_empty() && run_quiet("kill", &["-0", pid])? {
        Ok(format!("running (pid {pid})"))
    } else {
        Ok("stopped (stale pid file)".into())
    }
}

fn report(failures: &mut Vec<String>, label: String, success: bool) {
    println!("{:5} {label}", if success { "ok" } else { "FAIL" });
    if !success {
        failures.push(label);
    }
}

fn print_service(name: &str, healthy: bool) {
    println!(
        "{name:16} {}",
        if healthy { "healthy" } else { "unavailable" }
    );
}

const fn component_name(component: LogComponent) -> &'static str {
    match component {
        LogComponent::Web => "web",
        LogComponent::Daemon => "daemon",
        LogComponent::Oidc => "oidc",
        LogComponent::Zot => "zot",
    }
}
