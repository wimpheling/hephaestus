fn accepted(request: Request) {
    let session_id = Uuid::from_slice(&request.sid).expect("validated sid");
    let handoff = parse_handoff_secret(&request.handoff_secret);
    let response = CreateResponse::new(OpaqueId::from(session_id));
    tracing::info!(?session_id, "recorded opaque identifier");
    unrelated_macro!(handoff);
    let _ = (handoff, response);
}

impl Worker {
    fn captures(request: Request) {
        let secret = request.handoff_secret;
        let _ = secret;
    }

    fn unrelated() {
        tracing::info!(secret = "safe", "unrelated message");
    }
}
