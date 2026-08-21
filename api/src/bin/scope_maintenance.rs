use scope_postgres::db::{
    apply_maintenance_migrations, migration_plan, verify_schema, verify_writer_fence_available,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: scope-maintenance <plan|fence|apply|verify>"))?;
    if std::env::args().nth(2).is_some() {
        anyhow::bail!("usage: scope-maintenance <plan|fence|apply|verify>");
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
        "fence" => {
            verify_writer_fence_available(database_url).await?;
            println!(r#"{{"available":true}}"#);
        }
        "verify" => {
            verify_schema(database_url).await?;
            println!(r#"{{"exact":true}}"#);
        }
        _ => anyhow::bail!("usage: scope-maintenance <plan|fence|apply|verify>"),
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
