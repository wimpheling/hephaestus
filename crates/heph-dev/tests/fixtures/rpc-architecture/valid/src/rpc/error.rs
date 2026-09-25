use connectrpc::ConnectError;

fn map_error() -> ConnectError {
    ConnectError::internal("internal service error")
}

fn router() {
    let _ = axum::Router::new().route("/healthz", axum::routing::get(handler));
}
