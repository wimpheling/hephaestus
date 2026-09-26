//! Test composition boundary for authenticated gateway service-log RPC probes.
//!
//! Generated Connect clients and protobuf messages are deliberately contained here.
//! Callers receive only plain IDs, bytes, and test-owned proof data.

use identity_domain::BrowserSessionSid;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
use time::OffsetDateTime;

#[cfg(feature = "test-fixtures")]
#[derive(Debug)]
/// Proof returned after the public service-log request and authenticated read.
pub struct GatewayServiceGuestLogProof {
    /// Project scope used for the retained records.
    pub(super) project_id: uuid::Uuid,
    /// Gateway scope used for the retained records.
    pub(super) gateway_id: uuid::Uuid,
    /// Immutable service revision scope used for the retained records.
    pub(super) revision_id: uuid::Uuid,
    /// Service instance that emitted the records.
    pub(super) instance_id: uuid::Uuid,
    /// Fencing epoch associated with the records.
    pub(super) fencing_token: i64,
    /// Concatenated stdout bytes observed before shutdown.
    pub(super) stdout: Vec<u8>,
    /// Concatenated stderr bytes observed before shutdown.
    pub(super) stderr: Vec<u8>,
}

#[cfg(feature = "test-fixtures")]
/// Ordinary stdout marker emitted by the cooking fixture.
pub const GUEST_SERVICE_LOG_STDOUT_MARKER: &[u8] = b"service-log-stdout=ordinary\n";
#[cfg(feature = "test-fixtures")]
/// Ordinary stderr marker emitted by the cooking fixture.
pub const GUEST_SERVICE_LOG_STDERR_MARKER: &[u8] = b"service-log-stderr=ordinary\n";

#[cfg(feature = "test-fixtures")]
#[derive(Debug)]
/// Plain test data prepared by the golden SQL fixture before transport checks.
pub struct GatewayServiceLogRpcFixture {
    /// Gateway scope used by the RPC request.
    pub(super) gateway_id: uuid::Uuid,
    /// Immutable revision scope used by the RPC request.
    pub(super) revision_id: uuid::Uuid,
    /// Service instance scope used by the RPC request.
    pub(super) instance_id: uuid::Uuid,
    /// Project scope used by the RPC request.
    pub(super) project_id: uuid::Uuid,
    /// Fencing epoch used by the RPC request.
    pub(super) fencing_token: i64,
    /// First sequence expected after the seeded records.
    pub(super) first_sequence: i64,
    /// Epoch acknowledgement watermark before the seeded records.
    pub(super) baseline_acknowledged: i64,
    /// Retained bytes before the seeded records.
    pub(super) baseline_bytes: i64,
    /// Retained chunks before the seeded records.
    pub(super) baseline_chunks: i64,
    /// Seeded payloads returned by the paged RPC.
    pub(super) payloads: [Vec<u8>; 2],
    /// Total bytes in the seeded payloads.
    pub(super) payload_bytes: i64,
    /// Foreign project used by the authorization denial assertion.
    pub(super) foreign_project: uuid::Uuid,
    /// Member whose access is revoked between transport assertions.
    pub(super) member_id: uuid::Uuid,
    /// Stable random SID seeded for the member before RPC calls.
    pub(super) member_browser_session: BrowserSessionSid,
}

#[cfg(feature = "test-fixtures")]
fn opaque_id(value: uuid::Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

#[cfg(feature = "test-fixtures")]
fn service_log_rpc_token(actor: &uuid::Uuid, sid: BrowserSessionSid, method: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": actor.to_string(),
            "aud": format!("/hephaestus.gateway.v1.GatewayService/{method}"),
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": uuid::Uuid::new_v4().to_string(),
            "sid": sid.to_protocol_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign service log RPC mediator token")
}

#[path = "gateway_service_log/guest.rs"]
mod guest;
#[path = "gateway_service_log/rpc.rs"]
mod rpc;

pub use guest::{exercise_gateway_service_guest_log, marker_count};
pub use rpc::exercise_gateway_service_log_rpc;
