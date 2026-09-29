//! Authorization-aware read-only inspection and narrowly scoped recovery CLI.

use sqlx::postgres::PgPoolOptions;
use std::{env, error::Error};

#[path = "hephaestus_operator/catalog.rs"]
mod catalog;
#[path = "hephaestus_operator/catalog_validation.rs"]
mod catalog_validation;
#[path = "hephaestus_operator/cli.rs"]
mod cli;
#[path = "hephaestus_operator/inspection.rs"]
mod inspection;
#[path = "hephaestus_operator/recovery.rs"]
mod recovery;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    let output = cli::execute(&pool, &arguments).await?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
#[path = "hephaestus_operator/tests/operator.rs"]
mod tests;
