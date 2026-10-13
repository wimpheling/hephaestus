//! Cross-language hash vectors are data proof, never execution/issuance evidence.
use super::harness::Harness;
use capability_domain::RuntimeSessionIdentity;
use sqlx::PgPool;
use time::{Date, Duration, OffsetDateTime, Time};

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn actual_core_sql_hashes_unicode_full_bindings_and_signed128_timestamp_boundaries(
    pool: PgPool,
) {
    let mut h = Box::pin(Harness::new(&pool, 2)).await;
    h.add_optional_bindings().await;
    assert_eq!(
        usize::BITS,
        64,
        "this qualified encoder supports the reviewed Core framing"
    );
    let sql_snapshot: Vec<u8> = sqlx::query_scalar("SELECT invocation119_snapshot_hash($1,$2)")
        .bind(h.birth.start.run_id.as_uuid())
        .bind(h.snapshot.authorization_model_version())
        .fetch_one(&h.birth.birth.worker)
        .await
        .unwrap();
    assert_eq!(
        sql_snapshot.as_slice(),
        h.snapshot.normalized_hash().as_bytes()
    );
    let minimum = Date::MIN.with_time(Time::MIDNIGHT).assume_utc();
    let maximum = Date::MAX
        .with_hms_micro(23, 59, 59, 999_999)
        .unwrap()
        .assume_utc();
    let before = h.rows().await;
    for (issued, expires) in [
        (minimum, minimum + Duration::seconds(1)),
        (
            OffsetDateTime::from_unix_timestamp_nanos(-1_000).unwrap(),
            OffsetDateTime::UNIX_EPOCH,
        ),
        (
            OffsetDateTime::UNIX_EPOCH,
            OffsetDateTime::UNIX_EPOCH + Duration::microseconds(1),
        ),
        (maximum - Duration::seconds(1), maximum),
    ] {
        for timestamp in [issued, expires] {
            let nanoseconds = timestamp.unix_timestamp_nanos();
            // Owner-role fixture read only: no worker helper ACL is added.
            // This compares actual Core signed claims, not parsed SQL dates.
            let sql_bytes: Vec<u8> =
                sqlx::query_scalar("SELECT invocation119_i128($1::text::numeric)")
                    .bind(nanoseconds.to_string())
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                sql_bytes.as_slice(),
                nanoseconds.to_be_bytes().as_slice(),
                "actual Core signed128 bytes differ at ns={nanoseconds}"
            );
        }
        let identity = RuntimeSessionIdentity::new(
            h.identity.id(),
            h.identity.principal(),
            h.identity.invocation(),
            &h.snapshot,
            issued,
            expires,
        )
        .unwrap();
        let sql_identity: Vec<u8> = sqlx::query_scalar(
            "SELECT invocation119_identity_hash($1,$2,$3::text::numeric,$4::text::numeric)",
        )
        .bind(h.birth.start.run_id.as_uuid())
        .bind(sql_snapshot.as_slice())
        .bind(issued.unix_timestamp_nanos().to_string())
        .bind(expires.unix_timestamp_nanos().to_string())
        .fetch_one(&h.birth.birth.worker)
        .await
        .unwrap();
        assert_eq!(
            sql_identity.as_slice(),
            identity.normalized_hash().as_bytes(),
            "actual Core signed128 identity differs at {issued}"
        );
    }
    assert_eq!(
        (-1_i128).div_euclid(1000) * 1000,
        -1000,
        "negative floor before constructing Core identity"
    );
    assert_eq!(
        h.rows().await,
        before,
        "vectors do not issue sessions or prove physical runtime"
    );
    h.birth.physical.assert_unchanged();
}
