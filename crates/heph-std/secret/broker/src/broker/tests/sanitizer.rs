// Secret broker test scenarios.
use super::support::*;

#[test]
fn sanitizer_rejects_oversized_and_malicious_upstream_responses() {
    let oversized =
        vec![
            b'x';
            usize::try_from(MAX_UPSTREAM_RESPONSE_BYTES).expect("response bound fits usize") + 1
        ];
    assert!(matches!(
        sanitize_upstream_response(&oversized, b"credential"),
        Err(BrokerAdapterError::Rejected)
    ));
    for response in [
        b"HTTP/1.1 200 OK\r\nX-Injected: yes\r\n\r\n{\"result\":\"line\\nfeed\"}".as_slice(),
        b"HTTP/1.0 200 OK\r\n\r\n{\"result\":\"accepted\"}".as_slice(),
        b"HTTP/1.1 200 OK\r\n\r\n{\"result\":\"accepted\",\"token\":\"leak\"}".as_slice(),
    ] {
        assert!(matches!(
            sanitize_upstream_response(response, b"credential"),
            Err(BrokerAdapterError::Rejected)
        ));
    }
}
