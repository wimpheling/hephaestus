use super::*;
use release_domain::NetworkAccess;
use uuid::Uuid;

const ROOT: &str = "/prospective/nonexistent/provider";
const VERSION: &str = "platform/v1";

fn make(
    root: &str,
    version: &str,
) -> Result<InstanceExecutionConfiguration, InstanceExecutionError> {
    InstanceExecutionConfiguration::new(
        VmProviderOwnerScope::new("actual-owner".to_owned(), "host".to_owned()).unwrap(),
        root.to_owned(),
        VolumeRootNamespaceId::from_uuid(Uuid::from_u128(1)).unwrap(),
        RuntimePolicy {
            vcpus: 1,
            memory_mib: 512,
            network: NetworkAccess::Disabled,
        },
        version.to_owned(),
    )
}

// Frozen pre-promotion PostgreSQL encoder, retained independently as a byte oracle.
fn legacy_encoder(configuration: &InstanceExecutionConfiguration) -> Value {
    json!({
        "version":1,"profile":"runtime_named_v1",
        "vm":{"namespace":configuration.scope.namespace(),"host_id":configuration.scope.host_id()},
        "volume":{"namespace":configuration.volume_namespace.as_uuid(),"host_id":configuration.scope.host_id(),"root":configuration.volume_root},
        "platform":{"policy":configuration.platform_policy,"version":configuration.platform_version}
    })
}

#[test]
fn canonical_bytes_equal_the_legacy_encoder_and_fixed_vector() {
    let configured = make(ROOT, VERSION).unwrap();
    let actual = serde_json::to_vec(&configured.canonical_json()).unwrap();
    assert_eq!(
        actual,
        serde_json::to_vec(&legacy_encoder(&configured)).unwrap()
    );
    assert_eq!(
        actual,
        br#"{"platform":{"policy":{"memory_mib":512,"network":"disabled","vcpus":1},"version":"platform/v1"},"profile":"runtime_named_v1","version":1,"vm":{"host_id":"host","namespace":"actual-owner"},"volume":{"host_id":"host","namespace":"00000000-0000-0000-0000-000000000001","root":"/prospective/nonexistent/provider"}}"#
    );
}

#[test]
fn configuration_bounds_and_canonical_paths_do_not_perform_io() {
    for root in [
        "/",
        "relative",
        "/tmp/../foreign",
        "/tmp//foreign",
        "/tmp/./foreign",
        "/tmp/foreign/",
        "/tmp/unsafe\n",
    ] {
        assert!(matches!(
            make(root, VERSION),
            Err(InstanceExecutionError::ConfigurationConflict)
        ));
    }
    for version in ["", " padded", "unsafe\n"] {
        assert!(matches!(
            make(ROOT, version),
            Err(InstanceExecutionError::ConfigurationConflict)
        ));
    }
    assert!(
        make(ROOT, VERSION).is_ok(),
        "checked data is not physical root discovery"
    );
}

#[test]
fn exact_root_and_version_byte_bounds_are_preserved() {
    let root = format!("/{}", "a".repeat(4095));
    let version = "v".repeat(128);
    assert!(make(&root, &version).is_ok());
    assert!(matches!(
        make(&(root + "a"), VERSION),
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
    assert!(matches!(
        make(ROOT, &(version + "v")),
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
}

#[test]
fn checked_configuration_preserves_distinct_vm_and_volume_namespaces() {
    for namespace in ["actual-owner", "local:owner-v1", "provider.root_2"] {
        let configured = InstanceExecutionConfiguration::new(
            VmProviderOwnerScope::new(namespace.to_owned(), "host-1".to_owned()).unwrap(),
            ROOT.to_owned(),
            VolumeRootNamespaceId::from_uuid(Uuid::from_u128(1)).unwrap(),
            make(ROOT, VERSION).unwrap().platform_policy().clone(),
            VERSION.to_owned(),
        )
        .unwrap();
        let wire = configured.canonical_json();
        assert_eq!(configured.provider_scope().namespace(), namespace);
        assert_eq!(configured.provider_scope().host_id(), "host-1");
        assert_eq!(wire["vm"]["namespace"], namespace);
        assert_eq!(
            wire["volume"]["namespace"],
            configured.volume_namespace().as_uuid().to_string()
        );
        assert_ne!(wire["vm"]["namespace"], wire["volume"]["namespace"]);
        assert_eq!(configured.volume_root(), ROOT);
        assert_eq!(configured.platform_version(), VERSION);
    }
    assert!(VolumeRootNamespaceId::from_uuid(Uuid::nil()).is_err());
    assert!(VmProviderOwnerScope::new("a/b".to_owned(), "host".to_owned()).is_err());
}

#[test]
fn invalid_allocation_is_rejected_without_policy_clamping() {
    for (vcpus, memory_mib) in [(0, 512), (1, 0)] {
        let configured = make(ROOT, VERSION).unwrap();
        assert!(matches!(
            InstanceExecutionConfiguration::new(
                configured.scope,
                configured.volume_root,
                configured.volume_namespace,
                RuntimePolicy {
                    vcpus,
                    memory_mib,
                    network: NetworkAccess::Disabled
                },
                configured.platform_version,
            ),
            Err(InstanceExecutionError::ConfigurationConflict)
        ));
    }
    for network in [
        NetworkAccess::Disabled,
        NetworkAccess::BrokerOnly,
        NetworkAccess::Egress,
    ] {
        let original = make(ROOT, VERSION).unwrap();
        let policy = RuntimePolicy {
            vcpus: u8::MAX,
            memory_mib: u32::MAX,
            network,
        };
        let configured = InstanceExecutionConfiguration::new(
            original.scope,
            original.volume_root,
            original.volume_namespace,
            policy.clone(),
            original.platform_version,
        )
        .unwrap();
        assert_eq!(configured.platform_policy(), &policy);
        assert_eq!(
            serde_json::to_vec(&configured.canonical_json()).unwrap(),
            serde_json::to_vec(&legacy_encoder(&configured)).unwrap()
        );
    }
}
