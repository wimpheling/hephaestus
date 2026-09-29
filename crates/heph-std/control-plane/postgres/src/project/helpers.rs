use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::types::{
    InstanceRow, Page, PageResult, ProjectError, ProjectRepositoryImagePreparationEventRow,
    ProjectRepositoryImageRow, ProjectRepositoryRow, ReleaseAgentRow,
};

pub(super) async fn require_permission(
    tx: &mut Transaction<'_, Postgres>,
    permission: &str,
    kind: &str,
    id: Uuid,
) -> Result<(), ProjectError> {
    let allowed: bool = sqlx::query_scalar(
        "SELECT check_permission('user', hephaestus_actor_id(), $1, $2, $3::text) = 1",
    )
    .bind(permission)
    .bind(kind)
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(ProjectError::Persistence)?;
    if allowed {
        Ok(())
    } else {
        Err(ProjectError::PermissionDenied)
    }
}

pub(super) fn finish_page<T>(mut rows: Vec<T>, page: Page) -> Result<PageResult<T>, ProjectError>
where
    T: RowId,
{
    let take = usize::try_from(page.size).map_err(|_| ProjectError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next = has_more.then(|| rows.last().map(RowId::id)).flatten();
    Ok(PageResult { values: rows, next })
}

pub(super) trait RowId {
    fn id(&self) -> String;
}

macro_rules! row_id {
    ($($ty:ty),+ $(,)?) => {$(
        impl RowId for $ty {
            fn id(&self) -> String { self.id.to_string() }
        }
    )+};
}

row_id!(
    ProjectRepositoryRow,
    ProjectRepositoryImageRow,
    ProjectRepositoryImagePreparationEventRow,
    InstanceRow,
    ReleaseAgentRow
);
