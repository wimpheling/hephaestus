use super::support::*;
use crate::{
    MAX_REGISTRY_INVENTORY_ENTRIES, RegistryInventory, RegistryInventoryDocument,
    RegistryInventoryEntry, RegistryNotificationBacklog, RegistryOperationalMetrics,
    RegistryRetentionReport, RegistryRetentionReportError, RegistryRetentionSnapshot,
};

#[test]
fn retention_report_is_bounded_provider_neutral_and_non_destructive() {
    let publishing = intent().begin_publishing().expect("publishing");
    let verified = verification(publishing.reference());
    let approved = publishing
        .record_verified(verified)
        .expect("verified")
        .approve()
        .expect("approved");
    let namespace = approved.reference().namespace().clone();
    let inventory = RegistryInventory::try_from(RegistryInventoryDocument {
        schema_version: 1,
        entries: vec![
            RegistryInventoryEntry::new(namespace.clone(), digest(A)),
            RegistryInventoryEntry::new(namespace, digest('f')),
        ],
    })
    .expect("bounded inventory");
    let metrics = RegistryOperationalMetrics {
        approved_publications: 1,
        notification_backlog: RegistryNotificationBacklog {
            pending: 2,
            expired_claims: 1,
            ..RegistryNotificationBacklog::default()
        },
        ..RegistryOperationalMetrics::default()
    };

    let report = RegistryRetentionReport::evaluate(
        RegistryRetentionSnapshot::new(vec![approved], metrics),
        inventory,
    );
    let serialized = serde_json::to_value(report).expect("serialize report");

    assert_eq!(serialized["schema_version"], 1);
    assert_eq!(serialized["mode"], "report_only");
    assert_eq!(serialized["inventory_entries"], 2);
    assert_eq!(
        serialized["retention_roots"]
            .as_array()
            .expect("retention roots")
            .len(),
        5
    );
    assert_eq!(
        serialized["missing_from_inventory"]
            .as_array()
            .expect("missing roots")
            .len(),
        4
    );
    assert_eq!(
        serialized["unreferenced_inventory"]
            .as_array()
            .expect("unreferenced inventory")
            .len(),
        1
    );
    assert_eq!(serialized["observability"]["approved_publications"], 1);
    assert_eq!(
        serialized["observability"]["notification_backlog"]["pending"],
        2
    );
    assert_eq!(serialized["schema_scope"]["generic_build_images"], false);
}

#[test]
fn retention_inventory_rejects_unknown_or_unbounded_documents() {
    let unsupported = RegistryInventory::try_from(RegistryInventoryDocument {
        schema_version: 2,
        entries: Vec::new(),
    });
    assert_eq!(
        unsupported,
        Err(RegistryRetentionReportError::UnsupportedInventorySchema(2))
    );

    let entry = RegistryInventoryEntry::new(reference().namespace().clone(), digest(A));
    let oversized = RegistryInventory::try_from(RegistryInventoryDocument {
        schema_version: 1,
        entries: vec![entry; MAX_REGISTRY_INVENTORY_ENTRIES + 1],
    });
    assert_eq!(
        oversized,
        Err(RegistryRetentionReportError::InventoryTooLarge)
    );
}
