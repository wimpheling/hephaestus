pub fn list(page: i64) {
    let _ = sqlx::query(
        "SELECT items.id FROM items
          WHERE (items.created_at, items.id) < ($1, $2)
          ORDER BY items.created_at DESC, items.id DESC
          LIMIT $3",
    );
    let _ = page;
}
