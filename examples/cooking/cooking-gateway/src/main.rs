//! One-shot `http.v1` gateway fixture for the cooking journey.
//!
//! This executable deliberately knows only a symbolic mailbox slot. The
//! Hephaestus host resolves that slot to a mailbox and producer identity after
//! the handler returns; neither identity is present in this program.

use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const COOKING_REQUESTS_SLOT: &str = "cooking_requests";
const TELEGRAM_ROUTE: &str = "/gateway/cooking/telegram";
const GUEST_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Deserialize)]
struct Parameters {
    inbound_placeholder: String,
    alice_provider_id: u64,
    bob_provider_id: u64,
}

#[derive(Debug, Deserialize)]
struct PrivateHttpRequest {
    method: String,
    path_and_query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct PrivateHttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    mailbox_publication: Option<PrivateMailboxPublication>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct PrivateMailboxPublication {
    slot: String,
    method: String,
    route: String,
    headers: Vec<(String, String)>,
    content_type: Option<String>,
    trace_context: Option<String>,
    body: Vec<u8>,
    deduplication_key: String,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: u64,
    message: TelegramMessage,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    from: TelegramSender,
    text: String,
}

#[derive(Debug, Deserialize)]
struct TelegramSender {
    id: u64,
}

#[derive(Serialize)]
struct CookingEvent<'a> {
    provider_update_id: u64,
    user_id: &'a str,
    command: &'a str,
    text: &'a str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    io::stdin().take(131_073).read_to_end(&mut input)?;
    if input.len() > 131_072 {
        return Err("HTTP frame exceeds limit".into());
    }
    let request = ciborium::from_reader(input.as_slice())?;
    let parameters: Parameters =
        serde_json::from_slice(&std::fs::read("/run/hephaestus/parameters.json")?)?;
    if parameters.alice_provider_id == parameters.bob_provider_id
        || parameters.inbound_placeholder.is_empty()
    {
        return Err("invalid gateway parameters".into());
    }
    assert_gateway_boundary()?;
    let response = handle(&request, &parameters);
    ciborium::into_writer(&response, io::stdout())?;
    io::stdout().flush()?;
    Ok(())
}

/// The gateway has no workspace, state, or brokered provider mounts. Keep
/// this check in the released executable so a joined cooking request proves
/// those confinement decisions at the actual guest boundary.
fn assert_gateway_boundary() -> Result<(), &'static str> {
    for path in [
        "/workspace/repo/hugo.toml",
        "/var/lib/hephaestus/cooking.sqlite3",
        "/run/hephaestus-secrets/model",
        "/run/hephaestus-secrets/telegram_relay",
        "/workspace/repo/.git/HEAD",
    ] {
        if std::fs::read(path).is_ok() {
            return Err("gateway accessed a resource outside its declared mounts");
        }
    }
    // TEST-NET-2 is reserved for documentation and must never be a reachable
    // provider; only an explicit network-unreachable policy error passes.
    let address = SocketAddr::from(([198, 51, 100, 1], 443));
    match TcpStream::connect_timeout(&address, GUEST_PROBE_TIMEOUT) {
        Ok(_) => return Err("gateway direct network access is enabled"),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NetworkUnreachable | ErrorKind::PermissionDenied
            ) => {}
        Err(_) => return Err("gateway network denial was not authoritative"),
    }
    Ok(())
}

fn handle(request: &PrivateHttpRequest, parameters: &Parameters) -> PrivateHttpResponse {
    let path = request
        .path_and_query
        .split_once('?')
        .map_or(request.path_and_query.as_str(), |(path, _)| path);
    if request.method != "POST" || path != TELEGRAM_ROUTE {
        return response(404, b"not found", None);
    }

    let verification: Vec<_> = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("x-telegram-bot-api-secret-token"))
        .collect();
    if verification.len() != 1 || verification[0].1 != parameters.inbound_placeholder {
        return response(401, b"unauthorized", None);
    }
    if request.body.len() > 16_384 {
        return response(400, b"invalid Telegram update", None);
    }

    let Ok(update) = serde_json::from_slice::<TelegramUpdate>(&request.body) else {
        return response(400, b"invalid Telegram update", None);
    };
    if update.update_id > i64::MAX as u64 {
        return response(400, b"invalid Telegram update", None);
    }

    let text = update.message.text.trim();
    if text.is_empty() || text.len() > 2048 || text.chars().any(|c| c.is_control() && c != '\n') {
        return response(400, b"invalid Telegram update", None);
    }
    let user_id = match update.message.from.id {
        id if id == parameters.alice_provider_id => "alice",
        id if id == parameters.bob_provider_id => "bob",
        _ => return response(403, b"forbidden", None),
    };
    let text = text.strip_prefix("/recipe ").unwrap_or(text).trim();
    if text.is_empty() {
        return response(400, b"invalid Telegram update", None);
    }
    let event = CookingEvent {
        provider_update_id: update.update_id,
        user_id,
        command: "recipe",
        text,
    };
    let publication = PrivateMailboxPublication {
        slot: COOKING_REQUESTS_SLOT.to_owned(),
        method: "POST".to_owned(),
        route: "/telegram/updates".to_owned(),
        // This is application metadata, not copied inbound provider headers.
        headers: vec![(
            "x-cooking-event-kind".to_owned(),
            "cooking.telegram.received.v1".to_owned(),
        )],
        content_type: Some("application/json".to_owned()),
        trace_context: None,
        body: serde_json::to_vec(&event).expect("bounded event serialization"),
        deduplication_key: format!("telegram-update-{}", update.update_id),
    };
    response(200, b"accepted", Some(publication))
}

fn response(
    status: u16,
    body: &[u8],
    mailbox_publication: Option<PrivateMailboxPublication>,
) -> PrivateHttpResponse {
    PrivateHttpResponse {
        status,
        headers: vec![(
            "content-type".to_owned(),
            "text/plain; charset=utf-8".to_owned(),
        )],
        body: body.to_vec(),
        mailbox_publication,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters() -> Parameters {
        Parameters {
            inbound_placeholder: "cooking-telegram-placeholder".to_owned(),
            alice_provider_id: 1001,
            bob_provider_id: 1002,
        }
    }

    #[test]
    fn denies_invalid_auth_identity_and_bounds_without_publication() {
        let mut request = PrivateHttpRequest {
            method: "POST".to_owned(),
            path_and_query: TELEGRAM_ROUTE.to_owned(),
            headers: Vec::new(),
            body: br#"{"update_id":42,"message":{"from":{"id":9999},"text":"pasta"}}"#.to_vec(),
        };
        let denied = handle(&request, &parameters());
        assert_eq!(denied.status, 401);
        assert!(denied.mailbox_publication.is_none());
        request.headers.push((
            "x-telegram-bot-api-secret-token".to_owned(),
            "cooking-telegram-placeholder".to_owned(),
        ));
        let denied = handle(&request, &parameters());
        assert_eq!(denied.status, 403);
        assert!(denied.mailbox_publication.is_none());
        request.body = vec![b'x'; 16_385];
        let denied = handle(&request, &parameters());
        assert_eq!(denied.status, 400);
        assert!(denied.mailbox_publication.is_none());
        request.headers.push(request.headers[0].clone());
        assert_eq!(handle(&request, &parameters()).status, 401);
    }

    #[test]
    fn bob_command_is_normalized_and_retries_have_the_same_key() {
        let request = PrivateHttpRequest {
            method: "POST".to_owned(),
            path_and_query: TELEGRAM_ROUTE.to_owned(),
            headers: vec![(
                "x-telegram-bot-api-secret-token".to_owned(),
                "cooking-telegram-placeholder".to_owned(),
            )],
            body: br#"{"update_id":43,"message":{"from":{"id":1002},"text":" /recipe soup "}}"#
                .to_vec(),
        };
        let first = handle(&request, &parameters());
        assert_eq!(first, handle(&request, &parameters()));
        let body: serde_json::Value =
            serde_json::from_slice(&first.mailbox_publication.expect("publication").body)
                .expect("JSON");
        assert_eq!(body["user_id"], "bob");
        assert_eq!(body["text"], "soup");
    }

    #[test]
    fn accepts_a_telegram_update_with_a_stable_deduplication_key() {
        let response = handle(
            &PrivateHttpRequest {
                method: "POST".to_owned(),
                path_and_query: "/gateway/cooking/telegram?ignored=true".to_owned(),
                headers: vec![(
                    "x-telegram-bot-api-secret-token".to_owned(),
                    "cooking-telegram-placeholder".to_owned(),
                )],
                body: br#"{"update_id":42,"message":{"from":{"id":1001},"text":"pasta"}}"#.to_vec(),
            },
            &parameters(),
        );

        assert_eq!(response.status, 200);
        let publication = response.mailbox_publication.expect("publication");
        assert_eq!(publication.slot, COOKING_REQUESTS_SLOT);
        assert_eq!(publication.deduplication_key, "telegram-update-42");
        assert_eq!(publication.trace_context, None);
        assert_eq!(
            publication.body,
            br#"{"provider_update_id":42,"user_id":"alice","command":"recipe","text":"pasta"}"#
        );
    }

    #[test]
    fn rejects_a_payload_without_a_telegram_update_id() {
        let response = handle(
            &PrivateHttpRequest {
                method: "POST".to_owned(),
                path_and_query: TELEGRAM_ROUTE.to_owned(),
                headers: vec![(
                    "x-telegram-bot-api-secret-token".to_owned(),
                    "cooking-telegram-placeholder".to_owned(),
                )],
                body: br#"{"message":{"text":"pasta"}}"#.to_vec(),
            },
            &parameters(),
        );

        assert_eq!(response.status, 400);
        assert!(response.mailbox_publication.is_none());
    }
}
