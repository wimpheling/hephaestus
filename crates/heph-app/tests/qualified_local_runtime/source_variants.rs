//! Owned source changes precede the genuine build and immutable publication.

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use agent_config::{AgentConfig, parse};
use capability_domain::CapabilitySlotKey;
use tempfile::TempDir;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

const DATA: &str = "[[volume_slots]]\nslot = \"data\"\nguest_path = \"/data\"\naccess_mode = \"read_write\"\nrequired = true\nminimum_capacity_bytes = 16777216\n\n";
const AUDIT: &str = "[[volume_slots]]\nslot = \"audit\"\nguest_path = \"/audit\"\naccess_mode = \"read_write\"\nrequired = true\nminimum_capacity_bytes = 16777216\n\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeVariant {
    Zero,
    Two,
}
impl VolumeVariant {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Zero => "qualified-local-zero",
            Self::Two => "qualified-local-two",
        }
    }
    pub fn slots(self) -> Vec<VolumeSlotDeclaration> {
        let names: &[(&str, &str)] = match self {
            Self::Zero => &[],
            Self::Two => &[("audit", "/audit"), ("data", "/data")],
        };
        names
            .iter()
            .map(|(slot, path)| {
                VolumeSlotDeclaration::new(
                    CapabilitySlotKey::parse(*slot).unwrap(),
                    GuestMountPath::parse(*path).unwrap(),
                    VolumeAccessMode::ReadWrite,
                    true,
                    16_777_216,
                )
                .unwrap()
            })
            .collect()
    }
}

pub struct OwnedSourceVariant {
    directory: TempDir,
    pub variant: VolumeVariant,
    pub configuration: AgentConfig,
}
impl OwnedSourceVariant {
    pub fn create(canonical: &Path, owned_root: &Path, variant: VolumeVariant) -> Self {
        let source = fs::read_to_string(canonical.join("agent.toml")).unwrap();
        assert_eq!(source.matches(DATA).count(), 1);
        let mut expected = parse(source.as_bytes()).config.unwrap();
        assert_eq!(expected.guest.command, "bin/local-sqlite");
        assert_eq!(expected.guest.working_directory, "bin");
        assert!(!expected.workspace.mount && !expected.state_volume.enabled);
        assert_eq!(expected.volume_slots.len(), 1);
        let replacement = match variant {
            VolumeVariant::Zero => String::new(),
            VolumeVariant::Two => format!("{DATA}{AUDIT}"),
        };
        let changed = source.replacen(DATA, &replacement, 1);
        let configuration = parse(changed.as_bytes()).config.unwrap();
        expected
            .volume_slots
            .clone_from(&configuration.volume_slots);
        assert_eq!(configuration, expected, "only authored slots change");
        assert_eq!(
            configuration.effective_volume_slots().unwrap(),
            variant.slots()
        );
        let directory = tempfile::Builder::new()
            .prefix(variant.key())
            .tempdir_in(owned_root)
            .unwrap();
        fs::write(directory.path().join("agent.toml"), changed).unwrap();
        for file in ["app.py", "build.sh"] {
            fs::copy(canonical.join(file), directory.path().join(file)).unwrap();
            assert_eq!(
                fs::read(canonical.join(file)).unwrap(),
                fs::read(directory.path().join(file)).unwrap()
            );
            assert_eq!(
                fs::metadata(canonical.join(file))
                    .unwrap()
                    .permissions()
                    .mode(),
                fs::metadata(directory.path().join(file))
                    .unwrap()
                    .permissions()
                    .mode()
            );
        }
        assert_eq!(
            fs::read_to_string(canonical.join("agent.toml")).unwrap(),
            source
        );
        Self {
            directory,
            variant,
            configuration,
        }
    }
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}
