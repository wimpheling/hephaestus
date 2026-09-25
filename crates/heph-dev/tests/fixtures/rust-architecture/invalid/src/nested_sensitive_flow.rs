fn rejected_nested(request: Request) {
    let secret_value = request.secret.value;
    tracing::warn!(%secret_value, "received nested secret");
    let secret = request.secret;
    let aliased_value = secret.value;
    let json = serde_json::json!({"secret": aliased_value});
    append_application_event(secret_value);
    response.body(aliased_value);
    let safe = request.secret.label;
    tracing::info!(?safe, "safe nested field");
    let _ = json;
}
