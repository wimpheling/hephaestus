use super::*;

#[tokio::test]
async fn canonicalizes_request_and_strips_dynamic_connection_headers() {
    let (connection, mut server) = connection_pair();
    let server_task = tokio::spawn(async move {
        let request = read_request_headers(&mut server).await;
        let request = String::from_utf8(request).expect("request is HTTP text");
        assert!(request.starts_with("POST /gateway/store/item?source=test HTTP/1.1\r\n"));
        assert!(request.contains("host: edge.example.test\r\n"));
        assert!(request.contains("content-length: 7\r\n"));
        assert!(request.contains("connection: close\r\n"));
        assert!(request.contains("authorization: Bearer app-value\r\n"));
        assert!(!request.contains("host: attacker.example\r\n"));
        assert!(!request.contains("x-forwarded-for"));
        assert!(!request.contains("x-dynamic-hop"));
        server
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\nX-Service: yes\r\n\r\nok",
            )
            .await
            .expect("write response");
    });
    let mut request = request(b"payload");
    request
        .headers
        .insert(HOST, HeaderValue::from_static("attacker.example"));
    request.headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer app-value"),
    );
    request
        .headers
        .insert("x-forwarded-for", HeaderValue::from_static("198.51.100.1"));
    request.headers.insert(
        "connection",
        HeaderValue::from_static("x-dynamic-hop, Upgrade"),
    );
    request
        .headers
        .insert("x-dynamic-hop", HeaderValue::from_static("remove-me"));
    let response = exchange(connection, request, policy())
        .await
        .expect("bounded service exchange");
    server_task.await.expect("server task");
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, Bytes::from_static(b"ok"));
    assert_eq!(
        response.headers.get("x-service"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert_eq!(
        response.headers.get(CONTENT_LENGTH),
        Some(&HeaderValue::from_static("2"))
    );
    assert!(!response.headers.contains_key(CONNECTION));
}

#[tokio::test]
async fn preserves_percent_encoded_path_and_url_query() {
    let (connection, mut server) = connection_pair();
    let server_task = tokio::spawn(async move {
        let request = read_request_headers(&mut server).await;
        let request = String::from_utf8(request).expect("request is HTTP text");
        assert!(request.starts_with(
            "POST /gateway/store/item?next=%2Ffoo//bar&url=https://example.test/a?x=1 HTTP/1.1\r\n"
        ));
        server
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write response");
    });
    let mut request = request(b"");
    request.path_and_query =
        String::from("/gateway/store/item?next=%2Ffoo//bar&url=https://example.test/a?x=1");
    exchange(connection, request, policy())
        .await
        .expect("encoded query is a valid origin form");
    server_task.await.expect("server task");
}

#[tokio::test]
async fn accepts_chunked_responses_without_trailers() {
    let (connection, mut server) = connection_pair();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        server
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\n\r\n",
            )
            .await
            .expect("write chunked response");
    });
    let response = exchange(connection, request(b""), policy())
        .await
        .expect("chunked response is bounded");
    server_task.await.expect("server task");
    assert_eq!(response.body, Bytes::from_static(b"ok"));
}

#[tokio::test]
async fn accepts_identical_duplicate_content_length() {
    let (connection, mut server) = connection_pair();
    let server_task = tokio::spawn(async move {
        read_request_headers(&mut server).await;
        server
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            )
            .await
            .expect("write duplicate-length response");
    });
    let response = exchange(connection, request(b""), policy())
        .await
        .expect("identical content lengths are unambiguous");
    server_task.await.expect("server task");
    assert_eq!(response.body, Bytes::from_static(b"ok"));
}
