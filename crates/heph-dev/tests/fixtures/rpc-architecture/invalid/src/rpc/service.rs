use connectrpc::ConnectError;

impl ExampleService for ExampleRpc {
    fn get_example() {
        let _ = sqlx::query("SELECT 1");
        let _ = ConnectError::invalid_argument("bad request");
        let _ = axum::Router::new().route("/internal/v1/examples", axum::routing::get(handler));
    }
}
