use super::*;

#[test]
fn ui_key_accepts_bounded_lowercase_keys_only() {
    assert_eq!(
        UiKey::parse("assistant-v2").unwrap().as_str(),
        "assistant-v2"
    );
    assert!(UiKey::parse("").is_err());
    assert!(UiKey::parse("2assistant").is_err());
    assert!(UiKey::parse("assistant_v2").is_err());
    assert!(UiKey::parse("Assistant").is_err());
    assert!(UiKey::parse("assistant-").is_ok());
    assert!(UiKey::parse("a".repeat(64)).is_ok());
    assert!(UiKey::parse("a".repeat(65)).is_err());
}

#[test]
fn ui_route_path_rejects_urls_traversal_and_non_ascii() {
    for valid in ["index.html", "assets/app~v1/main.js", "V2/entry_file"] {
        assert!(UiRoutePath::parse(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "",
        "/index.html",
        "index.html/",
        "assets//main.js",
        "./index.html",
        "assets/../index.html",
        "assets/%2e%2e/index.html",
        "https://example.invalid/app",
        "//example.invalid/app",
        "app?next=other",
        "app#fragment",
        "app\\main",
        "app/é.html",
    ] {
        assert!(UiRoutePath::parse(invalid).is_err(), "{invalid}");
    }
    assert!(UiRoutePath::parse("a".repeat(256)).is_ok());
    assert!(UiRoutePath::parse("a".repeat(257)).is_err());
}

#[test]
fn ui_label_trims_and_rejects_controls_and_bidi_formatting() {
    assert_eq!(
        UiLabel::parse("  Déploiement  ").unwrap().as_str(),
        "Déploiement"
    );
    assert_eq!(
        UiLabel::parse("é".repeat(80)).unwrap().as_str(),
        "é".repeat(80)
    );
    assert!(UiLabel::parse(" ").is_err());
    assert!(UiLabel::parse("\nlabel").is_err());
    assert!(UiLabel::parse("label\t").is_err());
    assert!(UiLabel::parse("line\nfeed").is_err());
    assert!(UiLabel::parse("\u{202e}label").is_err());
    assert!(UiLabel::parse("label\u{2069}").is_err());
    assert!(UiLabel::parse("line\u{2028}separator").is_err());
    assert!(UiLabel::parse("line\u{2029}separator").is_err());
    assert!(UiLabel::parse("safe\u{202e}label").is_err());
    assert!(UiLabel::parse("é".repeat(81)).is_err());
}

#[test]
fn ui_enums_have_stable_wire_values() {
    assert_eq!(
        serde_json::to_string(&UiScope::Project).unwrap(),
        "\"project\""
    );
    assert_eq!(
        serde_json::to_string(&UiPresentation::FullPage).unwrap(),
        "\"full_page\""
    );
    assert_eq!(UiIcon::Chart.as_str(), "chart");
    assert_eq!(serde_json::to_string(&UiIcon::Chat).unwrap(), "\"chat\"");
    assert_eq!(UiMediaType::TextJavascript.as_str(), "text/javascript");
    assert_eq!(
        serde_json::to_string(&UiMediaType::ImageSvgXml).unwrap(),
        "\"image/svg+xml\""
    );
    assert!(serde_json::from_str::<UiMediaType>("\"text/html; charset=utf-8\"").is_err());
    assert_eq!(
        UiRepositoryGitAccess::default(),
        UiRepositoryGitAccess::None
    );
    assert_eq!(UiRepositoryGitAccess::Read.as_str(), "read");
    assert_eq!(
        UiRepositoryGitAccess::parse("read_write"),
        Ok(UiRepositoryGitAccess::ReadWrite)
    );
    assert!(UiRepositoryGitAccess::parse("write").is_err());
}

#[test]
fn ui_values_checked_deserialize_and_round_trip() {
    let key: UiKey = serde_json::from_str("\"assistant-v2\"").unwrap();
    let path: UiRoutePath = serde_json::from_str("\"assets/main.js\"").unwrap();
    let label: UiLabel = serde_json::from_str("\"Assistant\"").unwrap();
    assert_eq!(serde_json::to_string(&key).unwrap(), "\"assistant-v2\"");
    assert_eq!(serde_json::to_string(&path).unwrap(), "\"assets/main.js\"");
    assert_eq!(serde_json::to_string(&label).unwrap(), "\"Assistant\"");
    assert!(serde_json::from_str::<UiRoutePath>("\"../escape\"").is_err());
    assert!(serde_json::from_str::<UiKey>("\"BadKey\"").is_err());
    assert!(serde_json::from_str::<UiScope>("\"tenant\"").is_err());
}

#[test]
fn ui_schema_version_is_explicitly_closed() {
    assert_eq!(SCHEMA_VERSION, 1);
    assert!(validate_schema_version(SCHEMA_VERSION).is_ok());
    assert!(validate_schema_version(0).is_err());
    assert!(validate_schema_version(2).is_err());
}
