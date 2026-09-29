use control_plane_postgres::build::RequestBuild;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use std::fmt::Write as _;
use uuid::Uuid;

pub fn request(
    repository_id: Uuid,
    source_commit: &str,
    build_definition_hash: [u8; 32],
    configuration_hash: &agent_config::ConfigHash,
) -> RequestBuild {
    RequestBuild {
        repository_id,
        source_commit: source_commit.to_owned(),
        build_definition_hash,
        configuration_hash: decode_hash(configuration_hash.as_str()),
    }
}

pub fn decode_hash(value: &str) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        hash[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    hash
}

const fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

pub fn hex_hash(value: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

pub fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://manual-ui-build.example",
        format!("manual-ui-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
