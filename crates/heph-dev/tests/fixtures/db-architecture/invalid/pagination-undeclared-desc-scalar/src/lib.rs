pub fn list(page: i64) {
    let _ = sqlx::query(
        "SELECT items.id FROM items
          WHERE items.id < $1
          ORDER BY items.id DESC
          LIMIT $2",
    );
    let _ = page;
}
