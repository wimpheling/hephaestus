fn rejected(request: Request) {
    let _request_metadata = request.context;
    let secret = request.handoff_secret;
    tracing::info!(%secret, "received handoff");
    let _formatted = format!("handoff={:?}", secret);
    let _error = anyhow::anyhow!("invalid handoff: {}", secret);
    let _json = serde_json::json!({"handoff_secret": secret});
    append_application_event(secret);
    labels.insert("handoff_secret", secret);
    metrics.label("handoff_secret", secret);
    events.publish(secret);
    response.body(secret);
    let _error = Error::new(secret);
    let _text = secret.to_string();
    let _response = CreateResponse::new(secret);
}
