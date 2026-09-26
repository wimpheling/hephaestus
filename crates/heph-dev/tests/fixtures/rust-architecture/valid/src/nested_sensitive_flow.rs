fn accepted_nested(request: Request) {
    let opaque = Uuid::from_slice(&request.secret.value).expect("validated secret");
    let request_alias = request;
    let aliased = OpaqueId::from_bytes(&request_alias.secret.value);
    let secret = request.secret;
    let aliased_secret = OpaqueId::from_bytes(&secret.value);
    let safe = request.secret.label;
    let response = CreateResponse::new(OpaqueId::from_uuid(opaque));
    tracing::info!(?opaque, "recorded opaque identifier");
    let _ = (aliased, aliased_secret, safe, response);
}
