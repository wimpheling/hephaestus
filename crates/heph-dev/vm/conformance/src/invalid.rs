use super::{api::ProviderHarness, support::describe_result};
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
};
use vm_trait::{NetworkMode, PortForward, PortProtocol, RootFilesystem, VmError, VmId};

/// Verifies provider-neutral invalid specifications produce typed errors.
///
/// # Panics
///
/// Panics when an invalid specification is accepted or loses field context.
pub async fn invalid_core_specs_are_typed(harness: &impl ProviderHarness) {
    let provider = harness.provider();
    let mut cases = Vec::new();

    let mut empty_id = harness.long_running_spec("valid-empty-id");
    empty_id.id = VmId(String::new());
    cases.push(("id", empty_id));

    let mut zero_cpu = harness.long_running_spec("invalid-zero-cpu");
    zero_cpu.resources.vcpus = 0;
    cases.push(("resources.vcpus", zero_cpu));

    let mut zero_memory = harness.long_running_spec("invalid-zero-memory");
    zero_memory.resources.memory_mib = 0;
    cases.push(("resources.memory_mib", zero_memory));

    let mut relative_program = harness.long_running_spec("invalid-relative-program");
    relative_program.command.program = String::from("bin/true");
    cases.push(("command.program", relative_program));

    let mut relative_working_dir = harness.long_running_spec("invalid-relative-working-dir");
    relative_working_dir.command.working_dir = Some(PathBuf::from("workspace"));
    cases.push(("command.working_dir", relative_working_dir));

    let mut nul_argument = harness.long_running_spec("invalid-nul-argument");
    nul_argument.command.args.push("bad\0argument".to_owned());
    cases.push(("command.args", nul_argument));

    let mut invalid_environment = harness.long_running_spec("invalid-environment");
    invalid_environment
        .command
        .env
        .insert("BAD=KEY".to_owned(), "value".to_owned());
    cases.push(("command.env", invalid_environment));

    let mut relative_root = harness.long_running_spec("invalid-relative-root");
    relative_root.root = match relative_root.root {
        RootFilesystem::Directory { .. } => RootFilesystem::Directory {
            host_path: PathBuf::from("relative-root"),
        },
        RootFilesystem::Disk {
            format, read_only, ..
        } => RootFilesystem::Disk {
            host_path: PathBuf::from("relative-root"),
            format,
            read_only,
        },
        _ => panic!("test harness returned an unknown root filesystem"),
    };
    cases.push(("root.host_path", relative_root));

    let mut non_loopback = harness.long_running_spec("invalid-non-loopback");
    non_loopback.network = NetworkMode::UserMode {
        ingress: vec![PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            host_port: 8080,
            guest_port: 80,
        }],
    };
    cases.push(("network.ingress[0].bind_addr", non_loopback));

    let mut zero_guest_port = harness.long_running_spec("invalid-zero-guest-port");
    zero_guest_port.network = NetworkMode::UserMode {
        ingress: vec![PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            host_port: 8080,
            guest_port: 0,
        }],
    };
    cases.push(("network.ingress[0].guest_port", zero_guest_port));

    let mut duplicate_binding = harness.long_running_spec("invalid-duplicate-binding");
    let duplicate = PortForward {
        protocol: PortProtocol::Tcp,
        bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        host_port: 18080,
        guest_port: 80,
    };
    duplicate_binding.network = NetworkMode::UserMode {
        ingress: vec![duplicate.clone(), duplicate],
    };
    cases.push(("network.ingress[1]", duplicate_binding));

    for (expected_field, spec) in cases {
        let result = provider.provision(spec).await;
        assert!(
            matches!(
                result,
                Err(VmError::InvalidSpec { ref field, .. })
                    if field.starts_with(expected_field)
            ),
            "expected InvalidSpec for {expected_field}, received {}",
            describe_result(&result)
        );
    }
}
