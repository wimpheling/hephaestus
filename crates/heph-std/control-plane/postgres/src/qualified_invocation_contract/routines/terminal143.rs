//! Exact Source143 interfaces and protected terminal schema; no helper is executed.
use super::{Entry, Postgres, Transaction, invalid, verify_routine};
#[path = "terminal143/entries0.rs"]
mod entries0;
#[path = "terminal143/entries1.rs"]
mod entries1;
#[path = "terminal143/entries2.rs"]
mod entries2;
#[path = "terminal143/entries3.rs"]
mod entries3;
#[path = "terminal143/schema.rs"]
mod schema;
pub(super) const SOURCE: &str =
    include_str!("../../../../../../../migrations/0143_prepared_instance_terminal.sql");
fn entries() -> impl Iterator<Item = &'static Entry> {
    entries0::ENTRIES
        .iter()
        .chain(entries1::ENTRIES.iter())
        .chain(entries2::ENTRIES.iter())
        .chain(entries3::ENTRIES.iter())
}
pub(super) fn contains(name: &str) -> bool {
    entries().any(|entry| entry.name == name)
}
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let names: Vec<_> = entries().map(|entry| entry.name).collect();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname::text=ANY($1::text[])").bind(names).fetch_one(&mut **admin).await?;
    if count != 27 {
        return Err(invalid(
            "Source143 exact 16 new / 11 replaced routines differ",
        ));
    }
    for entry in entries() {
        verify_routine(admin, entry).await?;
    }
    schema::verify(admin).await
}
