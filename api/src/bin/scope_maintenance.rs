use scope_postgres::db::{
    apply_maintenance_migrations, migration_plan, terminate_metadata_writer_sessions,
    verify_schema, verify_writer_fence_available,
};

const USAGE: &str =
    "usage: scope-maintenance <plan|fence|drain-writers|apply|backfill-landing-files|verify>";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!(USAGE))?;
    if std::env::args().nth(2).is_some() {
        anyhow::bail!(USAGE);
    }
    let database_url = maintenance_database_url()?;

    match command.as_str() {
        "plan" => {
            println!(
                "{}",
                serde_json::to_string(&migration_plan(database_url).await?)?
            );
        }
        "apply" => {
            apply_maintenance_migrations(database_url.clone()).await?;
            verify_schema(database_url).await?;
            println!(r#"{{"exact":true,"migration":"applied"}}"#);
        }
        "backfill-landing-files" => {
            verify_schema(database_url).await?;
            let state = api::AppState::from_env().await?;
            let stored = state.backfill_repository_landing_files().await?;
            println!(r#"{{"landingFilesBackfilled":{stored}}}"#);
        }
        "fence" => {
            verify_writer_fence_available(database_url).await?;
            println!(r#"{{"available":true}}"#);
        }
        "drain-writers" => {
            let terminated = terminate_metadata_writer_sessions(database_url).await?;
            println!(r#"{{"terminated":{terminated}}}"#);
        }
        "verify" => {
            verify_schema(database_url).await?;
            println!(r#"{{"exact":true}}"#);
        }
        _ => anyhow::bail!(USAGE),
    }
    Ok(())
}

fn maintenance_database_url() -> anyhow::Result<String> {
    #[cfg(feature = "local-dev")]
    if api::dev::is_local_dev_env() {
        return api::dev::local_maintenance_database_url();
    }

    std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required for Scope metadata storage"))
}
