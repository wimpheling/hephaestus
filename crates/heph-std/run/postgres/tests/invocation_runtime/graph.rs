use capability_domain::CapabilitySlotKey;
use volume_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};

pub fn slots(count: usize) -> Vec<VolumeSlotDeclaration> {
    assert!(matches!(count, 0 | 2 | 32));
    (0..count)
        .map(|index| {
            VolumeSlotDeclaration::new(
                CapabilitySlotKey::parse(format!("data{index:02}")).unwrap(),
                GuestMountPath::parse(format!("/data/{index:02}")).unwrap(),
                VolumeAccessMode::ReadWrite,
                true,
                16_777_216,
            )
            .unwrap()
        })
        .collect()
}

pub fn declaration(f: &crate::support::Fixture, count: usize, sibling: bool) -> String {
    let mut parts = vec![
        "contract_version=1\nrecipe_id=\"invocation-lease-native\"\nrecipe_version=\"1.0.0\"\n[[inputs]]\nname=\"capacity\"\ndefault=16777216\nvalue_type={type=\"integer\",minimum=16777216,maximum=1073741824}\n".to_owned(),
    ];
    parts.extend((0..count).map(|index| format!(
        "[[resources]]\nkind=\"volume\"\nname=\"data{index:02}\"\nsource={{type=\"created\",capacity_bytes={{source=\"input\",name=\"capacity\"}}}}\nremoval=\"retain\"\n"
    )));
    let bindings = (0..count)
        .map(|index| format!("{{slot=\"data{index:02}\",resource=\"data{index:02}\",guest_path=\"/data/{index:02}\",access_mode=\"read_write\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    parts.push(format!(
        "[[resources]]\nkind=\"instance\"\nname=\"sqlite\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\nvolume_bindings=[{bindings}]\nremoval=\"retain\"\n",
        f.release,f.release_agent
    ));
    if sibling {
        assert_eq!(count, 0, "sibling is a real slotless prepared child");
        parts.push(format!(
            "[[resources]]\nkind=\"instance\"\nname=\"sibling\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\nvolume_bindings=[]\nremoval=\"retain\"\n",
            f.release, f.release_agent
        ));
    }
    parts.concat()
}
