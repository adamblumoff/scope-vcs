use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_object_store::{EncryptedObjectStore, ObjectStore, S3ObjectStore, S3ObjectStoreSettings};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const CURRENT_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_KEY";
const PREVIOUS_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_PREVIOUS_KEY";
const CONCURRENCY_ENV: &str = "SCOPE_OBJECT_REKEY_CONCURRENCY";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url = required_env("DATABASE_URL")?;
    let fence = scope_postgres::db::ExclusiveWriterFence::acquire(&database_url).await?;
    let result = tokio::task::spawn_blocking(rekey_objects).await?;
    let release_result = fence.release().await;
    result?;
    release_result?;
    Ok(())
}

fn rekey_objects() -> anyhow::Result<()> {
    let current_key = encryption_key(CURRENT_KEY_ENV)?;
    let previous_key = encryption_key(PREVIOUS_KEY_ENV)?;
    if current_key == previous_key {
        anyhow::bail!("current and previous object encryption keys must differ");
    }

    let concurrency = std::env::var(CONCURRENCY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(4);
    if !(1..=16).contains(&concurrency) {
        anyhow::bail!("{CONCURRENCY_ENV} must be between 1 and 16");
    }

    let raw = Arc::new(S3ObjectStore::new(s3_settings_from_env()?)?);
    let keys = raw.list_keys()?;
    eprintln!("found {} encrypted objects", keys.len());

    let rotating = Arc::new(EncryptedObjectStore::with_previous_key(
        raw.clone(),
        current_key,
        previous_key,
    ));
    let reencrypted = run_parallel(&keys, concurrency, |key| rotating.reencrypt(key))?;

    let verification_keys = raw.list_keys()?;
    let current = Arc::new(EncryptedObjectStore::new(raw, current_key));
    run_parallel(&verification_keys, concurrency, |key| {
        current.get(key).map(|_| false)
    })?;

    println!(
        "{{\"listed\":{},\"reencrypted\":{},\"verified\":{}}}",
        keys.len(),
        reencrypted,
        verification_keys.len()
    );
    Ok(())
}

fn run_parallel(
    keys: &[String],
    concurrency: usize,
    operation: impl Fn(&str) -> Result<bool, scope_object_store::ObjectStoreError> + Sync,
) -> anyhow::Result<usize> {
    let next = AtomicUsize::new(0);
    let processed = AtomicUsize::new(0);
    let changed = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..concurrency {
            let operation = &operation;
            let next = &next;
            let processed = &processed;
            let changed = &changed;
            let errors = &errors;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(key) = keys.get(index) else {
                        return;
                    };
                    match operation(key) {
                        Ok(was_changed) => {
                            changed.fetch_add(usize::from(was_changed), Ordering::Relaxed);
                        }
                        Err(error)
                            if error.kind == scope_object_store::ObjectStoreErrorKind::NotFound => {
                        }
                        Err(error) => {
                            errors.lock().unwrap().push(format!(
                                "object #{index} failed with {:?}: {}",
                                error.kind, error.message
                            ));
                        }
                    }
                    let completed = processed.fetch_add(1, Ordering::Relaxed) + 1;
                    if completed.is_multiple_of(250) || completed == keys.len() {
                        eprintln!("processed {completed}/{} objects", keys.len());
                    }
                }
            });
        }
    });

    let errors = errors.into_inner().unwrap();
    if !errors.is_empty() {
        anyhow::bail!(
            "{} object operations failed; first failure: {}",
            errors.len(),
            errors[0]
        );
    }
    Ok(changed.into_inner())
}

fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    let mut settings = S3ObjectStoreSettings::new(
        required_env("SCOPE_BUCKET_ENDPOINT")?,
        required_env("SCOPE_BUCKET_NAME")?,
        required_env("SCOPE_BUCKET_REGION")?,
        required_env("SCOPE_BUCKET_ACCESS_KEY_ID")?,
        required_env("SCOPE_BUCKET_SECRET_ACCESS_KEY")?,
    );
    settings.force_path_style = non_empty_env("SCOPE_BUCKET_FORCE_PATH_STYLE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    Ok(settings)
}

fn encryption_key(name: &str) -> anyhow::Result<[u8; 32]> {
    let encoded = required_env(name)?;
    let decoded = BASE64
        .decode(encoded.trim())
        .map_err(|error| anyhow::anyhow!("{name} must be base64: {error}"))?;
    decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("{name} must decode to exactly 32 bytes"))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
