pub fn list() {
    let _ = sqlx::query(
        "SELECT items.id FROM items
          WHERE (items.name, items.id) > ($1, $2)
          ORDER BY items.name ASC, items.id ASC
          LIMIT $3",
    );
}
