//! Pagination behavior checks for the service log reader.

use gateway_domain::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadRequest, GatewayServiceLogReadScope,
};
use gateway_postgres::PostgresGatewayServiceLogReader;
use identity_domain::AuthenticatedIdentity;

pub async fn verify_pagination(
    reader: &PostgresGatewayServiceLogReader,
    scope: GatewayServiceLogReadScope,
    owner: &AuthenticatedIdentity,
    first_cursor: GatewayServiceLogReadCursor,
) {
    let gap_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(
                scope,
                100,
                Some(GatewayServiceLogReadCursor::new(scope, 9).expect("gap boundary cursor")),
            )
            .expect("valid gap boundary request"),
        )
        .await
        .expect("gap boundary page");
    assert!(!gap_page.history_incomplete);
    assert_eq!(gap_page.records[0].sequence, 10);

    let second_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(first_cursor))
                .expect("valid continuation request"),
        )
        .await
        .expect("continuation page");
    assert_eq!(second_page.records.len(), 100);
    assert_eq!(second_page.records[0].sequence, 18);
    assert_eq!(second_page.records[99].sequence, 117);
    assert!(!second_page.history_incomplete);
    assert_eq!(
        second_page
            .next_after
            .expect("second continuation")
            .sequence(),
        117
    );

    let record_cursor = GatewayServiceLogReadCursor::new(scope, 19).expect("record cursor");
    let record_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(record_cursor))
                .expect("valid record-cap request"),
        )
        .await
        .expect("record-cap page");
    assert_eq!(record_page.records.len(), 100);
    assert_eq!(record_page.records[0].sequence, 20);
    assert_eq!(record_page.records[99].sequence, 119);
    assert_eq!(
        record_page
            .next_after
            .expect("record continuation")
            .sequence(),
        119
    );

    let final_cursor = GatewayServiceLogReadCursor::new(scope, 119).expect("final cursor");
    let final_page = reader
        .get_page(
            owner,
            GatewayServiceLogReadRequest::new(scope, 100, Some(final_cursor))
                .expect("valid final request"),
        )
        .await
        .expect("final page");
    assert_eq!(final_page.records.len(), 1);
    assert_eq!(final_page.records[0].sequence, 120);
    assert!(final_page.next_after.is_none());
}
