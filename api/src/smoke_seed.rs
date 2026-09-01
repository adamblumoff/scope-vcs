use crate::{
    auth::cli::CliAuthService,
    config::{data_dir, database_url_from_env, git_repo_root, non_empty_env},
    demo_seed::{DevSeedUser, catalog, seed_request_discussion_gallery, seed_user_account},
    object_store_config::{encryption_key_from_env, git_segment_store_from_env, s3_from_env},
    persistence::unix_now,
};
use scope_object_store::EncryptedObjectStore;
use scope_postgres::db::MetadataStore;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

const OPT_IN_ENV: &str = "SCOPE_ALLOW_STAGING_SMOKE_SEED";
const EXPECTED_PROJECT_ID_ENV: &str = "SCOPE_SMOKE_SEED_PROJECT_ID";
const EXPECTED_ENVIRONMENT_ID_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_ID";
const EXPECTED_ENVIRONMENT_NAME_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_NAME";
const PRODUCTION_ENVIRONMENT_ID_ENV: &str = "SCOPE_PRODUCTION_ENVIRONMENT_ID";
const SEED_USER_EMAIL_ENV: &str = "SCOPE_SMOKE_SEED_USER_EMAIL";
const SEED_USER_HANDLE_ENV: &str = "SCOPE_SMOKE_SEED_USER_HANDLE";
const EXCHANGE_TOKEN_PATH_ENV: &str = "SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH";

#[derive(Debug, PartialEq, Eq)]
struct Target {
    seed_user: DevSeedUser,
    exchange_token_path: PathBuf,
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
    exchange_token_path: Option<String>,
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
            exchange_token_path: non_empty_env(EXCHANGE_TOKEN_PATH_ENV),
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    let target = validate(&Snapshot::from_env())?;
    let mut exchange_token_file = create_exchange_token_file(&target.exchange_token_path)?;
    let seed_user = seed_user_account(target.seed_user.clone());
    let encryption_key = encryption_key_from_env()?;
    let s3 = tokio::task::spawn_blocking(s3_from_env).await??;
    let object_store = EncryptedObjectStore::new(Arc::new(s3), encryption_key);
    let local_root = data_dir(&git_repo_root()).join("git-segments");
    let git_segment_store = git_segment_store_from_env(local_root, encryption_key)?;
    let fixture = catalog(&object_store, &git_segment_store, target.seed_user)
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
    let now_unix = unix_now().map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    let grant = CliAuthService::new(metadata.auth())
        .create_staging_smoke_exchange_grant(&seed_user, now_unix)
        .await
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    write_exchange_token(&mut exchange_token_file, &grant.exchange_token)?;
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
    let exchange_token_path = PathBuf::from(required(
        EXCHANGE_TOKEN_PATH_ENV,
        &snapshot.exchange_token_path,
    )?);
    if !exchange_token_path.is_absolute() {
        anyhow::bail!("{EXCHANGE_TOKEN_PATH_ENV} must be an absolute path");
    }
    if exchange_token_path.file_name().is_none() {
        anyhow::bail!("{EXCHANGE_TOKEN_PATH_ENV} must identify a file");
    }

    Ok(Target {
        seed_user: DevSeedUser {
            email: email.to_string(),
            handle: handle.to_string(),
        },
        exchange_token_path,
    })
}

fn create_exchange_token_file(path: &Path) -> anyhow::Result<File> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{EXCHANGE_TOKEN_PATH_ENV} must have a parent directory"))?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| anyhow::anyhow!("inspecting exchange token directory: {error}"))?;
    if !parent_metadata.file_type().is_dir() {
        anyhow::bail!("staging smoke exchange token parent must be a directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if parent_metadata.permissions().mode() & 0o077 != 0 {
            anyhow::bail!(
                "staging smoke exchange token directory must not be accessible by group or other users"
            );
        }
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| anyhow::anyhow!("creating staging smoke exchange token file: {error}"))?;
    Ok(file)
}

fn write_exchange_token(file: &mut File, token: &str) -> anyhow::Result<()> {
    file.write_all(token.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| anyhow::anyhow!("writing staging smoke exchange token file: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|error| anyhow::anyhow!("reading exchange token file mode: {error}"))?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            anyhow::bail!("staging smoke exchange token file must have mode 0600");
        }
    }
    Ok(())
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
            exchange_token_path: Some("/tmp/scope-smoke/exchange-token".into()),
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
                exchange_token_path: PathBuf::from("/tmp/scope-smoke/exchange-token"),
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

    #[test]
    fn rejects_a_relative_exchange_token_path() {
        let mut snapshot = valid_snapshot();
        snapshot.exchange_token_path = Some("exchange-token".into());
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_exchange_token_file_once_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = temp.path().join("exchange-token");
        let mut file = create_exchange_token_file(&path).unwrap();
        write_exchange_token(&mut file, "scope_otc_test").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "scope_otc_test\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(create_exchange_token_file(&path).is_err());
    }
}
