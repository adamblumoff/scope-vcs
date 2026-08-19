use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use scope_cache_contract::SignedCacheGrantClaims;
use scope_cache_domain::{CacheDigest, RepositoryId};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct CacheGrantIssuer {
    endpoint: Arc<str>,
    backend: Arc<str>,
    key: Arc<EncodingKey>,
}

impl CacheGrantIssuer {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::new(
            required_env("SCOPE_CACHE_URL")?,
            required_env("SCOPE_CACHE_BACKEND")?,
            required_env("SCOPE_CACHE_GRANT_PRIVATE_KEY")?,
        )
    }

    fn new(endpoint: String, backend: String, private_key_pem: String) -> anyhow::Result<Self> {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        if !(endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1")) {
            anyhow::bail!("SCOPE_CACHE_URL must use HTTPS outside local development");
        }
        if backend.is_empty()
            || backend.len() > 64
            || backend.starts_with('-')
            || backend.ends_with('-')
            || backend.contains("--")
            || !backend
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            anyhow::bail!(
                "SCOPE_CACHE_BACKEND must contain lowercase letters, digits, or single hyphens"
            );
        }
        Ok(Self {
            endpoint: Arc::from(endpoint),
            backend: Arc::from(backend),
            key: Arc::new(EncodingKey::from_ed_pem(private_key_pem.as_bytes())?),
        })
    }

    pub(crate) fn issue(
        &self,
        repository_id: RepositoryId,
        allowed_identity_digests: Vec<CacheDigest>,
        expires_at_unix: u64,
    ) -> anyhow::Result<String> {
        Ok(encode(
            &Header::new(Algorithm::EdDSA),
            &SignedCacheGrantClaims {
                repository_id,
                allowed_identity_digests,
                backend: self.backend.to_string(),
                expires_at_unix,
            },
            &self.key,
        )?)
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    #[cfg(any(test, feature = "local-dev", feature = "test-support"))]
    pub(crate) fn test() -> Self {
        Self::new(
            "http://127.0.0.1:8082".to_string(),
            "test-local".to_string(),
            TEST_PRIVATE_KEY.to_string(),
        )
        .unwrap()
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";

#[cfg(test)]
pub(crate) const TEST_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    #[test]
    fn grant_is_signed_with_the_dedicated_ed25519_key() {
        let issuer = CacheGrantIssuer::test();
        let token = issuer
            .issue(
                RepositoryId::parse("repo-1").unwrap(),
                vec![CacheDigest::parse("a".repeat(64)).unwrap()],
                100,
            )
            .unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        let claims = decode::<SignedCacheGrantClaims>(
            &token,
            &DecodingKey::from_ed_pem(TEST_PUBLIC_KEY.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims.repository_id.as_str(), "repo-1");
        assert_eq!(claims.backend, "test-local");
        assert_eq!(claims.expires_at_unix, 100);
    }
}
