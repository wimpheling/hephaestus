use http::{
    HeaderMap, HeaderName, Method, Response, StatusCode,
    header::{self, CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING},
};

use super::{
    policy::ServiceHttpPolicy,
    request::{connection_nominated_headers, serialized_header_bytes},
};
use crate::GatewayEdgeError;

pub(super) fn validate_response_headers(
    response: &Response<hyper::body::Incoming>,
    policy: ServiceHttpPolicy,
) -> Result<(), GatewayEdgeError> {
    if response.headers().len() > policy.max_response_headers
        || serialized_header_bytes(response.headers()) > policy.max_wire_header_bytes
    {
        return Err(GatewayEdgeError::Contract(
            "service response headers exceed the limit",
        ));
    }
    Ok(())
}

pub(super) fn response_headers_without_transport(
    headers: &HeaderMap,
) -> Result<HeaderMap, GatewayEdgeError> {
    let nominated = connection_nominated_headers(headers)?;
    let mut headers = headers.clone();
    let names = headers.keys().cloned().collect::<Vec<_>>();
    for name in names {
        if nominated.contains(&name) {
            headers.remove(name);
        }
    }
    for name in [
        CONNECTION,
        HeaderName::from_static("keep-alive"),
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        HeaderName::from_static("proxy-connection"),
        header::TE,
        header::TRAILER,
        TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    Ok(headers)
}

#[derive(Clone, Copy)]
pub(super) struct ResponseFraming {
    content_length: Option<usize>,
    bodyless: bool,
    preserve_content_length: bool,
}

impl ResponseFraming {
    pub(super) const fn content_length(self) -> Option<usize> {
        self.content_length
    }

    pub(super) const fn is_bodyless(self) -> bool {
        self.bodyless
    }

    pub(super) const fn output_content_length(self, body_length: usize) -> Option<usize> {
        if self.preserve_content_length {
            self.content_length
        } else if self.bodyless {
            None
        } else {
            Some(body_length)
        }
    }
}

pub(super) fn response_framing(
    response: &Response<hyper::body::Incoming>,
    method: &Method,
) -> Result<ResponseFraming, GatewayEdgeError> {
    let status = response.status();
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return Err(GatewayEdgeError::Contract(
            "service HTTP upgrades are unsupported",
        ));
    }
    let content_length = parse_content_length(response.headers())?;
    let mut encodings = Vec::new();
    let transfer_encoding = response.headers().get_all(TRANSFER_ENCODING);
    for value in &transfer_encoding {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid service response encoding"))?;
        encodings.extend(value.split(',').map(str::trim));
    }
    let bodyless = *method == Method::HEAD
        || status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    if bodyless {
        if status == StatusCode::NO_CONTENT
            && (content_length.is_some_and(|length| length != 0) || !encodings.is_empty())
        {
            return Err(GatewayEdgeError::Contract(
                "invalid body framing for a 204 service response",
            ));
        }
        if !encodings.is_empty() && content_length.is_some() {
            return Err(GatewayEdgeError::Contract(
                "ambiguous service response framing",
            ));
        }
        return Ok(ResponseFraming {
            content_length,
            bodyless: true,
            preserve_content_length: *method == Method::HEAD || status == StatusCode::NOT_MODIFIED,
        });
    }
    if !encodings.is_empty() {
        if content_length.is_some()
            || encodings.len() != 1
            || !encodings[0].eq_ignore_ascii_case("chunked")
        {
            return Err(GatewayEdgeError::Contract(
                "ambiguous service response framing",
            ));
        }
        return Ok(ResponseFraming {
            content_length: None,
            bodyless: false,
            preserve_content_length: false,
        });
    }
    content_length.map_or_else(
        || {
            Err(GatewayEdgeError::Contract(
                "close-delimited service responses are unsupported",
            ))
        },
        |length| {
            Ok(ResponseFraming {
                content_length: Some(length),
                bodyless: false,
                preserve_content_length: false,
            })
        },
    )
}

fn parse_content_length(headers: &HeaderMap) -> Result<Option<usize>, GatewayEdgeError> {
    let mut length = None;
    for value in &headers.get_all(CONTENT_LENGTH) {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid service response length"))?;
        for part in value.split(',') {
            let parsed = part
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(GatewayEdgeError::Contract(
                    "invalid service response length",
                ))?;
            if length.is_some_and(|previous| previous != parsed) {
                return Err(GatewayEdgeError::Contract(
                    "ambiguous service response length",
                ));
            }
            length = Some(parsed);
        }
    }
    Ok(length)
}
