use sqlx::{QueryBuilder, query as imported_query};

pub async fn dynamic(pool: &sqlx::PgPool, statement: &str) -> Result<(), sqlx::Error> {
    sqlx::query(statement).execute(pool).await?;
    Ok(())
}

pub async fn imported(pool: &sqlx::PgPool, statement: &str) -> Result<(), sqlx::Error> {
    imported_query(statement).execute(pool).await?;
    Ok(())
}

pub fn builder(statement: &str) -> QueryBuilder<'_, sqlx::Postgres> {
    QueryBuilder::new(statement)
}

pub async fn schema(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("CREATE TABLE misplaced (id bigint)")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn included_schema(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(include_str!("schema_fragment.txt"))
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn query_file_schema(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_file!("query_schema.txt").execute(pool).await?;
    Ok(())
}
