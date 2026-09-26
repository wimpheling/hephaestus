use super::*;

#[path = "gateway_variants/authority.rs"]
mod authority;
#[path = "gateway_variants/routes.rs"]
mod routes;

pub(super) async fn verify(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    authority::verify(context, cases).await;
    routes::verify(context, cases).await;
}
