pub fn list() {
    let _ = sqlx::query(
        "SELECT items.id FROM items
          WHERE (items.name, items.id) > ($1, $2)
          LIMIT $3",
    );
}
