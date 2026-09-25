use super::outsider_token;
use identity_domain::{BrowserSessionSid, UserId};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

#[derive(serde::Deserialize)]
struct Claims {
    sid: String,
}

#[test]
fn outsider_token_binds_the_supplied_browser_session_sid() {
    let sid = BrowserSessionSid::new();
    let token = outsider_token(
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        UserId::new(),
        sid,
    );
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_aud = false;
    let claims = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
        &validation,
    )
    .expect("decode outsider mediator token")
    .claims;
    assert_eq!(claims.sid, sid.to_protocol_string());
}
