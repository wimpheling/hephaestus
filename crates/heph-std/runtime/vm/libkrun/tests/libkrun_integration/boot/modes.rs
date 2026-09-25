use std::time::Duration;

use runtime_types::RunId;
use secret_domain::{SecretSlotKey, SecretValue};
use secret_runtime::{EphemeralSecretConfig, RawSecretFile, materialize};
use vm_trait::{NetworkMode, StopMode, VmProvider};

use crate::{boot::BootContext, support::*};

pub async fn run(context: &BootContext) {
    let provider = &context.provider;
    let rootfs = context.rootfs.clone();
    let cgroup_root_for_assertion = context.cgroup_root.clone();
    let rootfs_for_graceful_test = context.rootfs_for_graceful_test.clone();
    let rootfs_for_force_test = context.rootfs_for_force_test.clone();
    let runtime_root = context.runtime_root.clone();
    let secret_path = context.secret_path.clone();
    let graceful = provider
        .provision(long_running_spec(rootfs_for_graceful_test, "graceful"))
        .await
        .expect("provision graceful-shutdown VM");
    let graceful_id = graceful.id().0.clone();
    graceful.start().await.expect("start graceful-shutdown VM");
    graceful
        .stop(StopMode::Graceful {
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("guest accepts graceful cancellation");
    let graceful_exit = graceful.wait().await.expect("graceful exit is cached");
    assert_eq!(graceful_exit.signal, Some(15));
    graceful.destroy().await.expect("cleanup graceful VM");
    assert!(!runtime_root.join(&graceful_id).exists());
    assert!(!cgroup_root_for_assertion.join(&graceful_id).exists());

    let mut forced_secret_mount = materialize(
        &EphemeralSecretConfig {
            root: secret_path
                .parent()
                .expect("primary secret mount has a parent")
                .to_path_buf(),
            require_memory_filesystem: false,
        },
        RunId::new(),
        vec![RawSecretFile {
            slot: SecretSlotKey::parse("forced_model").expect("forced secret slot"),
            value: SecretValue::new("libkrun-forced-secret-sentinel-91bd")
                .expect("forced secret fixture value"),
        }],
    )
    .expect("materialize forced-cleanup raw secret fixture");
    let forced_secret_path = forced_secret_mount.host_path().to_path_buf();
    let mut forced_spec = long_running_spec(rootfs_for_force_test.clone(), "force");
    forced_spec.mounts.push(forced_secret_mount.vm_mount());
    let forced = provider
        .provision(forced_spec)
        .await
        .expect("provision forced-cleanup VM");
    let forced_id = forced.id().0.clone();
    forced.start().await.expect("start forced-cleanup VM");
    forced.destroy().await.expect("force cleanup running VM");
    let forced_exit = forced.wait().await.expect("forced exit is cached");
    assert!(forced_exit.signal.is_some() || forced_exit.code.is_some());
    forced_secret_mount
        .mark_guest_destroyed()
        .expect("confirm forced guest destruction");
    forced_secret_mount
        .destroy()
        .expect("destroy forced-cleanup raw secret mount");
    assert!(
        !forced_secret_path.exists(),
        "raw secret mount survived forced guest destruction"
    );
    assert!(!runtime_root.join(&forced_id).exists());
    assert!(!cgroup_root_for_assertion.join(&forced_id).exists());

    let disabled = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-network-disabled",
            "--expect-network-disabled",
            NetworkMode::Disabled,
        ))
        .await
        .expect("provision disabled-network VM");
    let disabled_id = disabled.id().0.clone();
    let mut disabled_events = disabled.subscribe_events();
    disabled.start().await.expect("start disabled-network VM");
    let disabled_markers = collect_logs_until_exit(&mut disabled_events).await;
    assert!(disabled_markers.contains("network-disabled=ok"));
    disabled
        .destroy()
        .await
        .expect("destroy disabled-network VM");
    assert!(!runtime_root.join(&disabled_id).exists());
    assert!(!cgroup_root_for_assertion.join(&disabled_id).exists());
}
