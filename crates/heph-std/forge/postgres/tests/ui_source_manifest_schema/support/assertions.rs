pub fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected a database constraint or RLS error"),
        Err(error) => error,
    };
    let actual = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|code| code.to_string());
    assert_eq!(actual.as_deref(), Some(expected));
}
