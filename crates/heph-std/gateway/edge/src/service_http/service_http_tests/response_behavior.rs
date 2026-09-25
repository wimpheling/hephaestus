use super::*;

#[tokio::test]
async fn preserves_head_and_not_modified_lengths_but_drops_204_length() {
    let mut body_limited = policy();
    body_limited.max_response_body_bytes = 2;
    for (method, response_bytes, expected_length) in [
        (
            Method::HEAD,
            &b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n"[..],
            Some(9),
        ),
        (
            Method::GET,
            &b"HTTP/1.1 304 Not Modified\r\nContent-Length: 9\r\nConnection: close\r\n\r\n"[..],
            Some(9),
        ),
        (
            Method::GET,
            &b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..],
            None,
        ),
    ] {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            server
                .write_all(response_bytes)
                .await
                .expect("write bodyless response");
        });
        let response = exchange(connection, request_with_method(method, b""), body_limited)
            .await
            .expect("bodyless response is bounded");
        server_task.await.expect("server task");
        let actual_length = response.headers.get(CONTENT_LENGTH).map(|value| {
            std::str::from_utf8(value.as_bytes())
                .expect("length text")
                .parse::<usize>()
                .expect("length")
        });
        assert_eq!(actual_length, expected_length);
        assert!(response.body.is_empty());
    }
}

#[tokio::test]
async fn rejects_conflicting_content_length_and_transfer_encoding() {
    let (connection, mut server) = connection_pair();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        let _ = server
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n",
            )
            .await;
    });
    assert!(matches!(
        exchange(connection, request(b""), policy()).await,
        Err(GatewayEdgeError::Contract(
            "ambiguous service response framing"
        ))
    ));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn rejects_upgrades_even_without_a_switching_status() {
    for response_bytes in [
        &b"HTTP/1.1 200 OK\r\nUpgrade: websocket\r\nContent-Length: 0\r\n\r\n"[..],
        &b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n"[..],
    ] {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            let _ = server.write_all(response_bytes).await;
        });
        assert!(matches!(
            exchange(connection, request(b""), policy()).await,
            Err(GatewayEdgeError::Contract(_))
        ));
        server_task.await.expect("server task");
    }
}

#[tokio::test]
async fn rejects_trailers_and_close_delimited_responses() {
    for response_bytes in [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-Trailer\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\nX-Trailer: value\r\n\r\n"[..],
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\nX-Trailer: value\r\n\r\n"[..],
        &b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody"[..],
    ] {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            let _ = server.write_all(response_bytes).await;
        });
        assert!(matches!(
            exchange(connection, request(b""), policy()).await,
            Err(GatewayEdgeError::Contract(_))
        ));
        server_task.await.expect("server task");
    }
}

#[tokio::test]
async fn enforces_response_body_and_header_limits() {
    for (limits, expected) in [
        (
            ServiceHttpPolicy {
                max_response_body_bytes: 2,
                ..policy()
            },
            "body",
        ),
        (policy().with_max_wire_header_bytes(128), "header"),
    ] {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            server
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nX-One: 1\r\nX-Two: 123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890\r\n\r\ntest",
                )
                .await
                .expect("write oversized response");
        });
        let result = exchange(connection, request(b""), limits).await;
        assert!(
            matches!(result, Err(GatewayEdgeError::Contract(_))),
            "{expected} limit"
        );
        server_task.await.expect("server task");
    }
}

#[tokio::test]
async fn timeout_closes_the_private_stream() {
    let (connection, mut server) = connection_pair();
    let (headers_seen, headers_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        headers_seen.send(()).expect("signal request receipt");
        let mut byte = [0_u8; 1];
        let count = server.read(&mut byte).await.expect("observe stream close");
        assert_eq!(count, 0, "timed-out adapter kept the stream alive");
    });
    let mut short = policy();
    short.exchange_timeout = Duration::from_millis(25);
    let exchange_task = tokio::spawn(exchange(connection, request(b""), short));
    headers_received.await.expect("request reached peer");
    assert!(matches!(
        exchange_task.await.expect("exchange task"),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    server_task.await.expect("server task");
}

#[tokio::test]
async fn caller_cancellation_closes_the_private_stream() {
    let (connection, mut server) = connection_pair();
    let (headers_seen, headers_received) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        headers_seen.send(()).expect("signal request receipt");
        let mut byte = [0_u8; 1];
        let count = server.read(&mut byte).await.expect("observe stream close");
        assert_eq!(count, 0, "cancelled adapter kept the stream alive");
    });
    let exchange_task = tokio::spawn(exchange(connection, request(b""), policy()));
    headers_received.await.expect("request reached peer");
    exchange_task.abort();
    assert!(
        exchange_task
            .await
            .expect_err("exchange was cancelled")
            .is_cancelled()
    );
    server_task.await.expect("server task");
}

#[tokio::test]
async fn rejects_pathological_public_policy_values() {
    let (connection, _server) = connection_pair();
    let mut no_headers = policy();
    no_headers.max_response_headers = 0;
    assert!(matches!(
        exchange(connection, request(b""), no_headers).await,
        Err(GatewayEdgeError::Contract(_))
    ));

    let (connection, _server) = connection_pair();
    let mut unbounded_timeout = policy();
    unbounded_timeout.exchange_timeout = Duration::MAX;
    assert!(matches!(
        exchange(connection, request(b""), unbounded_timeout).await,
        Err(GatewayEdgeError::Contract(_))
    ));
}
