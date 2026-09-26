use gateway_domain::HttpMethod;

use super::*;

#[test]
fn raw_request_keeps_query_out_of_authority_path() {
    let request = UiBrowserHttpRequest::new(HttpMethod::Get, "/docs/app.js").expect("path");
    assert_eq!(request.path().as_str(), "/docs/app.js");
}

#[test]
fn raw_request_rejects_ambiguous_path_syntax() {
    for path in [
        "//docs/app.js",
        "/docs%2Fapp.js",
        "/docs/../app.js",
        "/docs/app.js?",
    ] {
        assert!(
            UiBrowserHttpRequest::new(HttpMethod::Get, path).is_err(),
            "{path}"
        );
    }
}

#[test]
fn raw_request_accepts_composed_static_path_bound() {
    let path = format!("/{}", "a".repeat(513));
    assert!(UiBrowserHttpRequest::new(HttpMethod::Get, path).is_ok());
    let too_long = format!("/{}", "a".repeat(514));
    assert!(UiBrowserHttpRequest::new(HttpMethod::Get, too_long).is_err());
}
