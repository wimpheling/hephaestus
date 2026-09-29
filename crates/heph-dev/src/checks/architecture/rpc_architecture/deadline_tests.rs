use super::{RULE, ServiceMethod, check_handler};
use crate::checks::architecture::Diagnostic;
use std::{fs, path::Path};

fn diagnostics(source: &str, stream: bool, test_only: bool) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let method = ServiceMethod {
        name: "ExampleService::load".to_owned(),
        source: format!("async fn load() {{ {source} }}"),
        line: 12,
        stream,
        delegation: None,
    };
    check_handler(
        Path::new("crates/heph-app/src/rpc/example.rs"),
        &method,
        test_only,
        &mut diagnostics,
    );
    diagnostics
}

#[test]
fn valid_unary_handler_has_one_budgeted_operation() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        let value = request::run_with_budget(&budget, service.application.load()).await?;
        let receipt = request::run_with_budget(&budget, mutation_receipt()).await??;
        let _ = (value, receipt);
    ";
    assert!(diagnostics(source, false, false).is_empty());
}

#[test]
fn missing_budget_reports_path_method_and_remediation() {
    let findings = diagnostics("service.application.load().await?;", false, false);
    let finding = findings
        .iter()
        .find(|finding| finding.rule_id == RULE)
        .expect("missing budget diagnostic");
    assert!(finding.message.contains("example.rs:12"));
    assert!(finding.message.contains("ExampleService::load"));
    assert!(finding.message.contains("RequestBudget"));
}

#[test]
fn unbounded_receipt_is_rejected_after_a_bounded_primary_call() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        let value = request::run_with_budget(&budget, service.application.load()).await?;
        let receipt = mutation_receipt().await?;
        let _ = (value, receipt);
    ";
    assert!(
        diagnostics(source, false, false)
            .iter()
            .any(|finding| finding.message.contains("outside a budget helper"))
    );
}

#[test]
fn unbounded_service_operation_is_rejected_after_a_bounded_primary_call() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        let value = request::run_with_budget(&budget, service.application.load()).await?;
        let follow_up = service.execute(command).await?;
        let _ = (value, follow_up);
    ";
    assert!(
        diagnostics(source, false, false)
            .iter()
            .any(|finding| finding.message.contains("outside a budget helper"))
    );
}

#[test]
fn helper_using_a_different_budget_is_rejected() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        let other_budget = request::RequestBudget::unbounded();
        let value = request::run_with_budget(&other_budget, service.application.load()).await?;
        let _ = (budget, value);
    ";
    assert!(
        diagnostics(source, false, false)
            .iter()
            .any(|finding| finding.message.contains("approved budgeted downstream"))
    );
}

#[test]
fn arbitrary_execute_helper_is_not_an_approved_budget_boundary() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        let value = execute(&budget, service.application.load()).await?;
        let _ = value;
    ";
    assert!(
        diagnostics(source, false, false)
            .iter()
            .any(|finding| finding.message.contains("approved budgeted downstream"))
    );
}

#[test]
fn braces_in_literal_do_not_confuse_the_ast_checker() {
    let source = r#"
        let budget = request::RequestBudget::from_transport(&ctx);
        let label = "literal with { braces }";
        let value = request::run_with_budget(&budget, service.application.load()).await?;
        let _ = (label, value);
    "#;
    assert!(diagnostics(source, false, false).is_empty());
}

#[test]
fn detached_work_requires_the_budget_cancellation_path() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        tokio::spawn(async move { service.application.load().await; });
        let _ = request::run_with_stream_budget(&budget, operation()).await;
    ";
    assert!(
        diagnostics(source, true, false)
            .iter()
            .any(|finding| finding.message.contains("detached work"))
    );
}

#[test]
fn detached_raw_send_is_rejected_even_with_a_cancellation_marker() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        tokio::spawn(async move {
            sender.send(item).await;
            let _ = budget.cancellation_token();
        });
    ";
    assert!(
        diagnostics(source, true, false)
            .iter()
            .any(|finding| finding.message.contains("detached work"))
    );
}

#[test]
fn detached_raw_receive_is_rejected_even_with_a_cancellation_marker() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        tokio::spawn(async move {
            let _ = receiver.recv().await;
            let _ = budget.cancellation_token();
        });
    ";
    assert!(
        diagnostics(source, true, false)
            .iter()
            .any(|finding| finding.message.contains("detached work"))
    );
}

#[test]
fn detached_raw_sleep_is_rejected_even_with_a_cancellation_marker() {
    let source = r"
        let budget = request::RequestBudget::from_transport(&ctx);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = budget.cancellation_token();
        });
    ";
    assert!(
        diagnostics(source, true, false)
            .iter()
            .any(|finding| finding.message.contains("detached work"))
    );
}

#[test]
fn test_only_handler_is_excluded() {
    let source = "service.application.load().await?;";
    assert!(diagnostics(source, false, true).is_empty());
}

#[test]
fn scanner_resolves_local_handler_delegation() {
    let root =
        std::env::temp_dir().join(format!("heph-rpc-deadline-fixture-{}", std::process::id()));
    let rpc = root.join("crates/heph-app/src/rpc");
    fs::create_dir_all(&rpc).expect("create fixture tree");
    fs::write(
        rpc.join("mod.rs"),
        r"
            impl ExampleService for ExampleRpc {
                async fn load(&self, ctx: RequestContext, request: Request) -> Result<(), ()> {
                    example::handle(self, ctx, request).await
                }
            }
        ",
    )
    .expect("write service fixture");
    fs::write(
        rpc.join("example.rs"),
        r"
            pub(super) async fn handle(
                service: &ExampleRpc,
                ctx: RequestContext,
                _request: Request,
            ) -> Result<(), ()> {
                let budget = request::RequestBudget::from_transport(&ctx);
                request::run_with_budget(&budget, service.application.load()).await?;
                Ok(())
            }
        ",
    )
    .expect("write handler fixture");

    let mut diagnostics = Vec::new();
    super::validate(&root, &[RULE], &mut diagnostics);
    fs::remove_dir_all(&root).expect("remove fixture tree");
    assert!(
        diagnostics.is_empty(),
        "unexpected diagnostics: {diagnostics:?}"
    );
}
