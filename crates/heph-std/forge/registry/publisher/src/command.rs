use std::process::{Command, Stdio};

use super::{
    config::{CommandOutput, CommandRunner, CommandSpec},
    errors::CommandRunnerError,
};

/// The real process runner used by the trusted publisher service.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

// `env_clear` is intentional for publisher subprocess isolation, but OCI
// tools may invoke standard helper binaries such as `newuidmap`.
const TRUSTED_SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput, CommandRunnerError> {
        let output = Command::new(&command.program)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .envs(command.environment.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .args(&command.arguments)
            .output()
            .map_err(|_| CommandRunnerError::LaunchFailed)?;
        if !output.status.success() {
            // The command specification redacts its bearer-token argument;
            // subprocess diagnostics contain the actionable registry error
            // without reproducing that secret. Keep them bounded so a failed
            // OCI client cannot flood the trusted release operator's logs.
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "trusted OCI command {} failed ({}): {}",
                command.program.display(),
                output.status,
                stderr.chars().take(4_096).collect::<String>().trim()
            );
        }
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}
