pub fn run(token: String) {
    let _ = vm_fake::FakeVm::default();
    let _ = std::env::var("TOKEN");
    let _ = reqwest::Client::new();
    let _ = std::process::Command::new("git");
    let _ = std::fs::read("secret.txt");
    tracing::info!(?token, "token");
    let _ = format!("token={:?}", token);
}
