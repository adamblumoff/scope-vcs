use crate::{
    config::{database_url_from_env, non_empty_env},
    demo_seed::{DevSeedUser, catalog, seed_request_discussion_gallery},
    object_store_config::{encryption_key_from_env, s3_from_env},
};
use scope_object_store::EncryptedObjectStore;
use scope_postgres::db::MetadataStore;
use std::sync::Arc;

const OPT_IN_ENV: &str = "SCOPE_ALLOW_STAGING_SMOKE_SEED";
const EXPECTED_PROJECT_ID_ENV: &str = "SCOPE_SMOKE_SEED_PROJECT_ID";
const EXPECTED_ENVIRONMENT_ID_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_ID";
const EXPECTED_ENVIRONMENT_NAME_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_NAME";
const PRODUCTION_ENVIRONMENT_ID_ENV: &str = "SCOPE_PRODUCTION_ENVIRONMENT_ID";
const SEED_USER_EMAIL_ENV: &str = "SCOPE_SMOKE_SEED_USER_EMAIL";
const SEED_USER_HANDLE_ENV: &str = "SCOPE_SMOKE_SEED_USER_HANDLE";

#[derive(Debug, PartialEq, Eq)]
struct Target {
    seed_user: DevSeedUser,
}

#[derive(Default)]
struct Snapshot {
    opt_in: Option<String>,
    expected_project_id: Option<String>,
    expected_environment_id: Option<String>,
    expected_environment_name: Option<String>,
    production_environment_id: Option<String>,
    actual_project_id: Option<String>,
    actual_environment_id: Option<String>,
    actual_environment_name: Option<String>,
    seed_user_email: Option<String>,
    seed_user_handle: Option<String>,
}

impl Snapshot {
    fn from_env() -> Self {
        Self {
            opt_in: non_empty_env(OPT_IN_ENV),
            expected_project_id: non_empty_env(EXPECTED_PROJECT_ID_ENV),
            expected_environment_id: non_empty_env(EXPECTED_ENVIRONMENT_ID_ENV),
            expected_environment_name: non_empty_env(EXPECTED_ENVIRONMENT_NAME_ENV),
            production_environment_id: non_empty_env(PRODUCTION_ENVIRONMENT_ID_ENV),
            actual_project_id: non_empty_env("RAILWAY_PROJECT_ID"),
            actual_environment_id: non_empty_env("RAILWAY_ENVIRONMENT_ID"),
            actual_environment_name: non_empty_env("RAILWAY_ENVIRONMENT_NAME"),
            seed_user_email: non_empty_env(SEED_USER_EMAIL_ENV),
            seed_user_handle: non_empty_env(SEED_USER_HANDLE_ENV),
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    let target = validate(&Snapshot::from_env())?;
    let s3 = tokio::task::spawn_blocking(s3_from_env).await??;
    let object_store = EncryptedObjectStore::new(Arc::new(s3), encryption_key_from_env()?);
    let fixture = catalog(&object_store, target.seed_user)
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    let metadata = MetadataStore::connect(database_url_from_env()?).await?;
    metadata
        .admin()
        .replace_catalog_for_seed(fixture)
        .await
        .map_err(|error| anyhow::anyhow!("replacing staging smoke catalog: {error}"))?;
    seed_request_discussion_gallery(&metadata)
        .await
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    println!(r#"{{"seeded":"dev/public-demo"}}"#);
    Ok(())
}

fn validate(snapshot: &Snapshot) -> anyhow::Result<Target> {
    require_exact(OPT_IN_ENV, snapshot.opt_in.as_deref(), "1")?;
    let expected_project_id = required(EXPECTED_PROJECT_ID_ENV, &snapshot.expected_project_id)?;
    let expected_environment_id = required(
        EXPECTED_ENVIRONMENT_ID_ENV,
        &snapshot.expected_environment_id,
    )?;
    let expected_environment_name = required(
        EXPECTED_ENVIRONMENT_NAME_ENV,
        &snapshot.expected_environment_name,
    )?;
    let production_environment_id = required(
        PRODUCTION_ENVIRONMENT_ID_ENV,
        &snapshot.production_environment_id,
    )?;
    if expected_environment_id == production_environment_id {
        anyhow::bail!("staging smoke seed target matches the production environment");
    }
    require_exact(
        "RAILWAY_PROJECT_ID",
        snapshot.actual_project_id.as_deref(),
        expected_project_id,
    )?;
    require_exact(
        "RAILWAY_ENVIRONMENT_ID",
        snapshot.actual_environment_id.as_deref(),
        expected_environment_id,
    )?;
    require_exact(
        "RAILWAY_ENVIRONMENT_NAME",
        snapshot.actual_environment_name.as_deref(),
        expected_environment_name,
    )?;
    if snapshot.actual_environment_id.as_deref() == Some(production_environment_id) {
        anyhow::bail!("refusing to replace the production catalog");
    }

    let email = required(SEED_USER_EMAIL_ENV, &snapshot.seed_user_email)?;
    if !email.contains('@') {
        anyhow::bail!("{SEED_USER_EMAIL_ENV} must be an email address");
    }
    let handle = required(SEED_USER_HANDLE_ENV, &snapshot.seed_user_handle)?;
    if !handle
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        anyhow::bail!("{SEED_USER_HANDLE_ENV} must contain only letters, numbers, or hyphens");
    }

    Ok(Target {
        seed_user: DevSeedUser {
            email: email.to_string(),
            handle: handle.to_string(),
        },
    })
}

fn required<'a>(name: &str, value: &'a Option<String>) -> anyhow::Result<&'a str> {
    value
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn require_exact(name: &str, actual: Option<&str>, expected: &str) -> anyhow::Result<()> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => anyhow::bail!("{name} does not match the reviewed staging target"),
        None => anyhow::bail!("{name} is required"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_snapshot() -> Snapshot {
        Snapshot {
            opt_in: Some("1".into()),
            expected_project_id: Some("project-staging".into()),
            expected_environment_id: Some("environment-staging".into()),
            expected_environment_name: Some("staging".into()),
            production_environment_id: Some("environment-production".into()),
            actual_project_id: Some("project-staging".into()),
            actual_environment_id: Some("environment-staging".into()),
            actual_environment_name: Some("staging".into()),
            seed_user_email: Some("smoke@example.test".into()),
            seed_user_handle: Some("dev".into()),
        }
    }

    #[test]
    fn accepts_the_reviewed_staging_target() {
        assert_eq!(
            validate(&valid_snapshot()).unwrap(),
            Target {
                seed_user: DevSeedUser {
                    email: "smoke@example.test".into(),
                    handle: "dev".into(),
                },
            }
        );
    }

    #[test]
    fn requires_explicit_opt_in() {
        let mut snapshot = valid_snapshot();
        snapshot.opt_in = None;
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains(OPT_IN_ENV)
        );
    }

    #[test]
    fn rejects_the_production_environment() {
        let mut snapshot = valid_snapshot();
        snapshot.actual_environment_id = snapshot.production_environment_id.clone();
        snapshot.expected_environment_id = snapshot.production_environment_id.clone();
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("production")
        );
    }

    #[test]
    fn rejects_a_different_project_or_environment() {
        for mutate in [
            |snapshot: &mut Snapshot| snapshot.actual_project_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_name = Some("wrong".into()),
        ] {
            let mut snapshot = valid_snapshot();
            mutate(&mut snapshot);
            assert!(validate(&snapshot).is_err());
        }
    }
}
