use scope_object_store::S3ObjectStoreSettings;

const DATABASE_URL: &str = "DATABASE_URL";
const CACHE_BUCKET_ENDPOINT: &str = "SCOPE_CACHE_BUCKET_ENDPOINT";
const CACHE_BUCKET_NAME: &str = "SCOPE_CACHE_BUCKET_NAME";
const CACHE_BUCKET_REGION: &str = "SCOPE_CACHE_BUCKET_REGION";
const CACHE_BUCKET_ACCESS_KEY_ID: &str = "SCOPE_CACHE_BUCKET_ACCESS_KEY_ID";
const CACHE_BUCKET_SECRET_ACCESS_KEY: &str = "SCOPE_CACHE_BUCKET_SECRET_ACCESS_KEY";
const CACHE_BUCKET_FORCE_PATH_STYLE: &str = "SCOPE_CACHE_BUCKET_FORCE_PATH_STYLE";
const CACHE_BACKEND: &str = "SCOPE_CACHE_BACKEND";
const CACHE_GRANT_PUBLIC_KEY: &str = "SCOPE_CACHE_GRANT_PUBLIC_KEY";

pub struct Settings {
    pub(crate) database_url: String,
    pub(crate) object_store: S3ObjectStoreSettings,
    pub(crate) backend: String,
    pub(crate) grant_public_key_pem: String,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let backend = required(CACHE_BACKEND)?;
        if backend.len() > 64
            || backend.starts_with('-')
            || backend.ends_with('-')
            || backend.contains("--")
            || !backend
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            anyhow::bail!(
                "{CACHE_BACKEND} must contain lowercase letters, digits, or single hyphens"
            );
        }
        let endpoint = required(CACHE_BUCKET_ENDPOINT)?;
        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1")) {
            anyhow::bail!("{CACHE_BUCKET_ENDPOINT} must use HTTPS outside local development");
        }
        let mut object_store = S3ObjectStoreSettings::new(
            endpoint,
            required(CACHE_BUCKET_NAME)?,
            required(CACHE_BUCKET_REGION)?,
            required(CACHE_BUCKET_ACCESS_KEY_ID)?,
            required(CACHE_BUCKET_SECRET_ACCESS_KEY)?,
        );
        object_store.force_path_style = optional(CACHE_BUCKET_FORCE_PATH_STYLE)
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        Ok(Self {
            database_url: required(DATABASE_URL)?,
            object_store,
            backend,
            grant_public_key_pem: required(CACHE_GRANT_PUBLIC_KEY)?,
        })
    }
}

fn required(name: &str) -> anyhow::Result<String> {
    optional(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}
