use super::common::{
    Bytes, ErrorSnapshot, HeaderMap, HeaderName, HeaderValue, StatusCode, VmError, watch,
};
use super::helpers::provider_error;

pub(super) fn private_http_request_message(
    request: vm_trait::PrivateHttpRequest,
) -> Result<crate::protocol::PrivateHttpRequestMessage, VmError> {
    if request.body.len() > crate::protocol::MAX_PRIVATE_HTTP_BODY_BYTES {
        return Err(invalid_private_http(
            "private HTTP request body exceeds provider limit".to_owned(),
        ));
    }
    let headers = private_http_headers(&request.headers, "request")?;
    let method = request.method.as_str();
    if method.is_empty() || method.bytes().any(|byte| !byte.is_ascii_uppercase()) {
        return Err(invalid_private_http(
            "private HTTP method must be uppercase ASCII".to_owned(),
        ));
    }
    if !request.path_and_query.starts_with('/') || request.path_and_query.contains('\0') {
        return Err(invalid_private_http(
            "private HTTP path must be an absolute non-NUL path".to_owned(),
        ));
    }
    Ok(crate::protocol::PrivateHttpRequestMessage {
        method: method.to_owned(),
        path_and_query: request.path_and_query,
        headers,
        body: request.body.to_vec(),
    })
}

pub(super) fn invalid_private_http(reason: impl Into<String>) -> VmError {
    VmError::InvalidSpec {
        field: "private_http".to_owned(),
        reason: reason.into(),
    }
}

pub(super) fn private_http_response(
    response: crate::protocol::PrivateHttpResponseMessage,
) -> Result<vm_trait::PrivateHttpResponse, VmError> {
    if response.body.len() > crate::protocol::MAX_PRIVATE_HTTP_BODY_BYTES {
        return Err(invalid_private_http(
            "private HTTP response body exceeds provider limit".to_owned(),
        ));
    }
    let status = StatusCode::from_u16(response.status)
        .map_err(|_| invalid_private_http("private HTTP response status is invalid".to_owned()))?;
    let headers = private_http_header_pairs(response.headers, "response")?;
    let mailbox_publication = response
        .mailbox_publication
        .map(private_mailbox_publication)
        .transpose()?;
    Ok(vm_trait::PrivateHttpResponse {
        status,
        headers,
        body: Bytes::from(response.body),
        mailbox_publication,
    })
}

pub(super) fn private_mailbox_publication(
    publication: crate::protocol::PrivateMailboxPublicationMessage,
) -> Result<vm_trait::PrivateMailboxPublication, VmError> {
    use crate::protocol::{
        MAX_MAILBOX_PUBLICATION_BODY_BYTES, MAX_MAILBOX_PUBLICATION_CONTENT_TYPE_BYTES,
        MAX_MAILBOX_PUBLICATION_DEDUPLICATION_KEY_BYTES, MAX_MAILBOX_PUBLICATION_HEADER_NAME_BYTES,
        MAX_MAILBOX_PUBLICATION_HEADER_VALUE_BYTES, MAX_MAILBOX_PUBLICATION_HEADERS,
        MAX_MAILBOX_PUBLICATION_METHOD_BYTES, MAX_MAILBOX_PUBLICATION_ROUTE_BYTES,
        MAX_MAILBOX_PUBLICATION_SLOT_BYTES, MAX_MAILBOX_PUBLICATION_TRACE_CONTEXT_BYTES,
    };
    let bounded =
        |value: &str, maximum| !value.is_empty() && value.len() <= maximum && !value.contains('\0');
    let valid_method = !publication.method.is_empty()
        && publication.method.len() <= MAX_MAILBOX_PUBLICATION_METHOD_BYTES
        && publication
            .method
            .bytes()
            .all(|byte| byte.is_ascii_uppercase());
    let valid_optional = |value: &Option<String>, maximum| {
        value
            .as_ref()
            .is_none_or(|value| value.len() <= maximum && !value.contains('\0'))
    };
    if !bounded(&publication.slot, MAX_MAILBOX_PUBLICATION_SLOT_BYTES)
        || !valid_method
        || !bounded(&publication.route, MAX_MAILBOX_PUBLICATION_ROUTE_BYTES)
        || !publication.route.starts_with('/')
        || publication.headers.len() > MAX_MAILBOX_PUBLICATION_HEADERS
        || publication.headers.iter().any(|(name, value)| {
            !bounded(name, MAX_MAILBOX_PUBLICATION_HEADER_NAME_BYTES)
                || value.len() > MAX_MAILBOX_PUBLICATION_HEADER_VALUE_BYTES
                || value.contains('\0')
        })
        || !valid_optional(
            &publication.content_type,
            MAX_MAILBOX_PUBLICATION_CONTENT_TYPE_BYTES,
        )
        || !valid_optional(
            &publication.trace_context,
            MAX_MAILBOX_PUBLICATION_TRACE_CONTEXT_BYTES,
        )
        || publication.body.len() > MAX_MAILBOX_PUBLICATION_BODY_BYTES
        || !bounded(
            &publication.deduplication_key,
            MAX_MAILBOX_PUBLICATION_DEDUPLICATION_KEY_BYTES,
        )
    {
        return Err(invalid_private_http(
            "mailbox publication violates provider limits".to_owned(),
        ));
    }
    Ok(vm_trait::PrivateMailboxPublication {
        slot: publication.slot,
        method: publication.method,
        route: publication.route,
        headers: publication.headers,
        content_type: publication.content_type,
        trace_context: publication.trace_context,
        body: Bytes::from(publication.body),
        deduplication_key: publication.deduplication_key,
    })
}

pub(super) fn private_http_headers(
    headers: &HeaderMap,
    direction: &str,
) -> Result<Vec<(String, String)>, VmError> {
    if headers.len() > crate::protocol::MAX_PRIVATE_HTTP_HEADERS {
        return Err(invalid_private_http(format!(
            "private HTTP {direction} has too many headers"
        )));
    }
    headers
        .iter()
        .map(|(name, value)| {
            let value = value.to_str().map_err(|_| {
                invalid_private_http(format!("private HTTP {direction} header is not UTF-8"))
            })?;
            if value.contains(['\r', '\n', '\0']) {
                return Err(invalid_private_http(format!(
                    "private HTTP {direction} header contains control characters"
                )));
            }
            Ok((name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

pub(super) fn private_http_header_pairs(
    headers: Vec<(String, String)>,
    direction: &str,
) -> Result<HeaderMap, VmError> {
    if headers.len() > crate::protocol::MAX_PRIVATE_HTTP_HEADERS {
        return Err(invalid_private_http(format!(
            "private HTTP {direction} has too many headers"
        )));
    }
    let mut parsed = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        if value.contains(['\r', '\n', '\0']) {
            return Err(invalid_private_http(format!(
                "private HTTP {direction} header contains control characters"
            )));
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            invalid_private_http(format!("private HTTP {direction} header name is invalid"))
        })?;
        let value = HeaderValue::from_str(&value).map_err(|_| {
            invalid_private_http(format!("private HTTP {direction} header value is invalid"))
        })?;
        parsed.append(name, value);
    }
    Ok(parsed)
}

pub(super) async fn wait_start_result(
    receiver: &mut watch::Receiver<Option<Result<(), ErrorSnapshot>>>,
) -> Result<(), VmError> {
    loop {
        let current = receiver.borrow_and_update().clone();
        if let Some(result) = current {
            return result.map_err(|error| error.to_vm_error());
        }
        receiver
            .changed()
            .await
            .map_err(|error| provider_error("start-channel", error))?;
    }
}
