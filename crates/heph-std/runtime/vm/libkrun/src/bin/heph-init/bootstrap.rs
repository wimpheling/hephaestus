use std::{
    io::{self},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use vm_libkrun::protocol::{
    GuestLogStream, GuestMessage, HostMessage, PROTOCOL_VERSION, RUNTIME_GIT_CREDENTIAL_HELPER,
    RUNTIME_GIT_HOST_ENV, RUNTIME_GIT_PATH_ENV,
};

use crate::{
    AGENT_GID, AGENT_UID, BUILD_GUEST_ENV, PLATFORM_OCI_BUILDER_ENV, PLATFORM_OCI_VERIFIER_ENV,
    connect_control, exit_parts, gateway_handler_loop, git_bridge, handle_host_messages,
    join_log_thread, mount_state_volume, mount_virtiofs, persist_runtime_authority,
    platform_oci_operation, provision_guest_open_files, pump_logs, read_frame, send_guest_error,
    service, unmount, wait_command, write_frame, write_message,
};

// The bootstrap sequence is intentionally kept in one ordered protocol flow.
#[allow(clippy::too_many_lines)]
pub fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut control = connect_control()?;
    write_frame(
        &mut control,
        &GuestMessage::Hello {
            version: PROTOCOL_VERSION,
        },
    )?;
    let HostMessage::Start {
        version,
        mut command,
        mounts,
        state_volume,
        runtime_authority,
        gateway_handler,
        private_http_service,
        runtime_git_bridge,
    } = read_frame(&mut control)?
    else {
        return Err("host did not send the start command".into());
    };
    if version != PROTOCOL_VERSION {
        return Err(format!("unsupported host protocol version {version}").into());
    }
    let service_config = private_http_service
        .as_ref()
        .map(service::ServiceConfig::try_from)
        .transpose()
        .inspect_err(|error| {
            send_guest_error(&mut control, "private-http-service", error);
        })?;
    let git_bridge_config = runtime_git_bridge
        .as_ref()
        .map(git_bridge::Config::try_from)
        .transpose()
        .inspect_err(|error| {
            send_guest_error(&mut control, "runtime-git-bridge", error);
        })?;
    if git_bridge_config.is_some()
        && runtime_authority
            .as_ref()
            .and_then(|authority| authority.runtime_git_credential.as_ref())
            .is_none()
    {
        let error = io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git bridge requires a runtime Git credential",
        );
        send_guest_error(&mut control, "runtime-git-bridge", &error);
        return Err(error.into());
    }
    if git_bridge_config.is_some() && gateway_handler {
        let error = io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git bridge cannot use a gateway handler",
        );
        send_guest_error(&mut control, "runtime-git-bridge", &error);
        return Err(error.into());
    }
    if service_config.is_some() && (gateway_handler || runtime_authority.is_some()) {
        let error = io::Error::new(
            io::ErrorKind::InvalidInput,
            "private HTTP services cannot use gateway handlers or runtime authority",
        );
        send_guest_error(&mut control, "private-http-service", &error);
        return Err(error.into());
    }

    // Persist the authority before mounting the immutable runtime control tree
    // at `/run/hephaestus`. A read-only nested virtiofs mount can otherwise
    // make its parent unsuitable for creating the sibling authority directory
    // on some libkrun/FUSE combinations.
    let runtime_authority_ack = if service_config.is_some() {
        None
    } else {
        runtime_authority
            .as_deref()
            .map(|authority| persist_runtime_authority(authority, &command))
            .transpose()?
    };

    for mount in mounts {
        if let Err(error) = mount_virtiofs(&mount.tag, &mount.guest_path, mount.read_only) {
            let error = io::Error::new(
                error.kind(),
                format!(
                    "mount {} at {}: {error}",
                    mount.tag,
                    mount.guest_path.display()
                ),
            );
            send_guest_error(&mut control, "mount", &error);
            return Err(error.into());
        }
    }
    let mounted_state = match state_volume.as_ref().map(mount_state_volume).transpose() {
        Ok(path) => path,
        Err(error) => {
            send_guest_error(&mut control, "state-volume", &error);
            return Err(error.into());
        }
    };
    if let Some(delay) = command.env.get("HEPH_TEST_READY_DELAY_MS") {
        let milliseconds = delay.parse::<u64>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid HEPH_TEST_READY_DELAY_MS: {error}"),
            )
        })?;
        thread::sleep(Duration::from_millis(milliseconds));
    }
    if let Some((session_id, generation)) = runtime_authority_ack {
        write_frame(
            &mut control,
            &GuestMessage::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            },
        )?;
    }

    let git_proxy = git_bridge_config
        .map(git_bridge::Supervisor::start)
        .transpose()?;
    if let Some(config) = git_bridge_config {
        let host = format!("127.0.0.1:{}", config.loopback_port);
        command.env.insert(String::from(RUNTIME_GIT_HOST_ENV), host);
        command.env.insert(
            String::from(RUNTIME_GIT_PATH_ENV),
            config.repository_id.to_string(),
        );
        // The host materializes the managed worktree before the guest starts,
        // so its numeric owner may differ from AGENT_UID. Restrict Git's
        // ownership exception to this one runtime-managed path.
        command
            .env
            .insert(String::from("GIT_CONFIG_COUNT"), String::from("3"));
        command.env.insert(
            String::from("GIT_CONFIG_KEY_0"),
            String::from("credential.helper"),
        );
        command.env.insert(
            String::from("GIT_CONFIG_VALUE_0"),
            String::from(RUNTIME_GIT_CREDENTIAL_HELPER),
        );
        command.env.insert(
            String::from("GIT_CONFIG_KEY_1"),
            String::from("credential.useHttpPath"),
        );
        command
            .env
            .insert(String::from("GIT_CONFIG_VALUE_1"), String::from("true"));
        command.env.insert(
            String::from("GIT_CONFIG_KEY_2"),
            String::from("safe.directory"),
        );
        command.env.insert(
            String::from("GIT_CONFIG_VALUE_2"),
            String::from("/workspace/git"),
        );
        command
            .env
            .insert(String::from("GIT_TERMINAL_PROMPT"), String::from("0"));
    }

    if gateway_handler {
        let writer = Arc::new(Mutex::new(control.try_clone()?));
        write_message(&writer, &GuestMessage::Ready)?;
        write_message(
            &writer,
            &GuestMessage::Metric {
                name: String::from("heph_init.ready"),
                value: 1.0,
                labels: std::collections::BTreeMap::from([(
                    String::from("protocol"),
                    PROTOCOL_VERSION.to_string(),
                )]),
            },
        )?;
        gateway_handler_loop(control, &writer, &command)?;
        if let Some(path) = mounted_state {
            unmount(&path)?;
        }
        return Ok(());
    }

    let platform_oci_operation = platform_oci_operation(&command);
    if platform_oci_operation.needs_large_fd_limit()
        || command
            .env
            .get(BUILD_GUEST_ENV)
            .is_some_and(|value| value == "1")
    {
        provision_guest_open_files()?;
    }
    if service_config.is_some() {
        if let Err(error) = service::bring_up_loopback() {
            send_guest_error(&mut control, "private-http-service-loopback", &error);
            return Err(error.into());
        }
    }
    let mut child = Command::new(&command.program);
    child
        .args(&command.args)
        .env_clear()
        .envs(command.env.iter().filter(|(key, _)| {
            !matches!(
                key.as_str(),
                PLATFORM_OCI_BUILDER_ENV | PLATFORM_OCI_VERIFIER_ENV | BUILD_GUEST_ENV
            )
        }))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(config) = service_config.as_ref() {
        child
            .env(service::SERVICE_HOST_ENV, service::SERVICE_HOST)
            .env(service::SERVICE_PORT_ENV, config.loopback_port.to_string());
    }
    if !platform_oci_operation.runs_as_root() {
        child.uid(AGENT_UID).gid(AGENT_GID);
    }
    if let Some(working_dir) = command.working_dir {
        child.current_dir(working_dir);
    }
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some(proxy) = git_proxy {
                proxy.stop();
            }
            send_guest_error(&mut control, "command-spawn", &error);
            return Err(error.into());
        }
    };

    let writer = Arc::new(Mutex::new(control.try_clone()?));
    write_message(&writer, &GuestMessage::Ready)?;
    write_message(
        &writer,
        &GuestMessage::Metric {
            name: String::from("heph_init.ready"),
            value: 1.0,
            labels: std::collections::BTreeMap::from([(
                String::from("protocol"),
                PROTOCOL_VERSION.to_string(),
            )]),
        },
    )?;
    let stdout = child
        .stdout
        .take()
        .ok_or("command stdout pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("command stderr pipe is unavailable")?;
    let stdout_thread = pump_logs(stdout, GuestLogStream::Stdout, Arc::clone(&writer));
    let stderr_thread = pump_logs(stderr, GuestLogStream::Stderr, Arc::clone(&writer));
    let service_supervisor = service_config.map(service::ServiceSupervisor::new);
    let control_thread = handle_host_messages(
        control,
        Arc::clone(&writer),
        child.id(),
        service_supervisor.clone(),
    );

    let status = wait_command(&mut child)?;
    if let Some(proxy) = git_proxy {
        proxy.stop();
    }
    if let Some(supervisor) = service_supervisor.as_ref() {
        supervisor.cancel();
    }
    join_log_thread(stdout_thread)?;
    join_log_thread(stderr_thread)?;
    if let Some(supervisor) = service_supervisor {
        supervisor.join_all();
    }
    let (code, signal) = exit_parts(status);
    if let Some(path) = mounted_state {
        unmount(&path)?;
    }
    if code == Some(0) && signal.is_none() {
        write_message(
            &writer,
            &GuestMessage::FinalizeResult {
                message: String::from("Hephaestus agent result"),
            },
        )?;
    }
    write_message(&writer, &GuestMessage::Exited { code, signal })?;
    drop(control_thread);
    Ok(())
}
