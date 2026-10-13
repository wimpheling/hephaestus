use super::{
    catalog, catalog_stability, fixture, journal, preservation,
    snapshots::{self, Phase, Snapshot},
};
use sqlx::PgPool;

pub(super) async fn replay143(
    pool: &PgPool,
    before: &Snapshot,
    before_journal: &journal::Journal<'_>,
    upgraded: &Snapshot,
    upgraded_journal: &journal::Journal<'_>,
    upgraded_catalog: &catalog::Snapshot,
) -> Result<(), sqlx::Error> {
    fixture::run143(pool).await;
    let replay = snapshots::capture(pool, Phase::Public143, Some(before)).await?;
    let replay_journal = journal::validate(&replay, Some(before_journal))?;
    upgraded_journal.assert_replay(&replay_journal)?;
    preservation::assert_replay(upgraded, &replay);
    let replay_catalog = catalog::capture(pool).await?;
    catalog_stability::assert_replay(upgraded_catalog, &replay_catalog);
    Ok(())
}

pub(super) async fn prefix137(
    pool: &PgPool,
    qualification: &super::Qualification<'_>,
    before: &Snapshot,
    before_journal: &journal::Journal<'_>,
    before_catalog: &catalog::Snapshot,
) -> Result<(), sqlx::Error> {
    fixture::run137(pool).await;
    let prefix = snapshots::capture(pool, Phase::Public137, Some(before)).await?;
    let prefix_catalog = catalog::capture(pool).await?;
    super::assert_upgrade(
        pool,
        qualification,
        &prefix,
        before_journal,
        before_catalog,
        &prefix_catalog,
    )
    .await?;
    Ok(())
}
