use uuid::Uuid;

use super::types::OrganizationError;

pub(super) fn truncate_page<T>(
    mut rows: Vec<T>,
    page_size: i64,
    id: impl Fn(&T) -> Uuid,
) -> Result<(Vec<T>, Option<String>), OrganizationError> {
    let take = usize::try_from(page_size).map_err(|_| OrganizationError::InvalidPage)?;
    let has_more = rows.len() > take;
    rows.truncate(take);
    let next_page_token = has_more
        .then(|| rows.last())
        .flatten()
        .map(|last| id(last).to_string());
    Ok((rows, next_page_token))
}
