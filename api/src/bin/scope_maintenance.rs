use scope_postgres::db::{
    apply_maintenance_migrations, migration_plan, terminate_metadata_writer_sessions,
    verify_schema, verify_writer_fence_available,
};

const USAGE: &str = r#"usage: scope-maintenance <command>

commands:
  plan                        print the pending migration plan as JSON (read-only)
  verify                      require the exact migration ledger (read-only)
  fence                       probe the exclusive writer fence (read-only)
  drain-writers               terminate sessions holding the shared writer fence
  validate-workflow-catalogs  validate pre-migration workflow inputs
  apply                       apply all pending migrations behind the writer fence
  backfill-git-segments-v2    rewrite legacy Git segments before the v2 migration
  backfill-landing-files      idempotently rebuild repository landing-file metadata
  backfill-workflow-catalogs  idempotently rebuild repository workflow catalogs
  help                        show this help

Production cutovers are owned by the backend deployment workflow. If apply may
have committed and recovery cannot prove the old ledger is unchanged, keep API
and worker writers closed and rerun the same revision to finish forward."#;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(|| anyhow::anyhow!(USAGE))?;
    if args.next().is_some() {
        anyhow::bail!(USAGE);
    }
    if matches!(command.as_str(), "help" | "-h" | "--help") {
        println!("{USAGE}");
        return Ok(());
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
        "backfill-git-segments-v2" => {
            let migrated = api::backfill_git_segments_v2_for_maintenance(database_url).await?;
            println!(r#"{{"gitSegmentsBackfilled":{migrated}}}"#);
        }
        "backfill-landing-files" => {
            verify_schema(database_url).await?;
            let state = api::AppState::from_env().await?;
            let stored = state.backfill_repository_landing_files().await?;
            println!(r#"{{"landingFilesBackfilled":{stored}}}"#);
        }
        "backfill-workflow-catalogs" => {
            verify_schema(database_url).await?;
            let state = api::AppState::from_env().await?;
            let stored = state.backfill_repository_workflow_catalogs().await?;
            println!(r#"{{"workflowCatalogsBackfilled":{stored}}}"#);
        }
        "fence" => {
            verify_writer_fence_available(database_url).await?;
            println!(r#"{{"available":true}}"#);
        }
        "drain-writers" => {
            let terminated = terminate_metadata_writer_sessions(database_url).await?;
            println!(r#"{{"terminated":{terminated}}}"#);
        }
        "validate-workflow-catalogs" => {
            let validated =
                api::validate_repository_workflow_catalogs_for_maintenance(database_url).await?;
            println!(r#"{{"workflowCatalogsValidated":{validated}}}"#);
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
