use crate::error::ServiceError;
use axum::http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use scope_cache_contract::SignedCacheGrantClaims;
use scope_cache_domain::CacheDigest;

pub(crate) struct GrantVerifier {
    key: DecodingKey,
    backend: String,
}

impl GrantVerifier {
    pub(crate) fn new(public_key_pem: &str, backend: String) -> anyhow::Result<Self> {
        if public_key_pem.trim().is_empty() {
            anyhow::bail!("cache grant public key is required");
        }
        Ok(Self {
            key: DecodingKey::from_ed_pem(public_key_pem.as_bytes())?,
            backend,
        })
    }

    pub(crate) fn verify(
        &self,
        headers: &HeaderMap,
        now_unix: u64,
    ) -> Result<SignedCacheGrantClaims, ServiceError> {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ServiceError::unauthorized("cache grant is required"))?;
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| ServiceError::unauthorized("cache grant is malformed"))?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        let claims = decode::<SignedCacheGrantClaims>(token, &self.key, &validation)
            .map_err(|_| ServiceError::unauthorized("cache grant is invalid"))?
            .claims;
        if claims.backend != self.backend || now_unix >= claims.expires_at_unix {
            return Err(ServiceError::unauthorized(
                "cache grant is expired or targets another backend",
            ));
        }
        Ok(claims)
    }
}

pub(crate) fn require_cache(
    claims: &SignedCacheGrantClaims,
    exact_identity: &CacheDigest,
    compatibility_group: &CacheDigest,
    now_unix: u64,
) -> Result<(), ServiceError> {
    if !claims.allows_cache(exact_identity, compatibility_group, now_unix) {
        return Err(ServiceError::forbidden(
            "cache grant does not allow this exact identity and compatibility group",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use jsonwebtoken::{EncodingKey, Header, encode};
    use scope_cache_contract::AuthorizedCache;
    use scope_cache_domain::RepositoryId;

    #[test]
    fn verifier_enforces_signature_backend_expiry_and_identity() {
        let verifier = GrantVerifier::new(TEST_PUBLIC_KEY, "test-local".to_string()).unwrap();
        let identity = CacheDigest::parse("a".repeat(64)).unwrap();
        let group = CacheDigest::parse("c".repeat(64)).unwrap();
        let claims = SignedCacheGrantClaims {
            attempt_id: "attempt-1".to_string(),
            repository_id: RepositoryId::parse("repo-1").unwrap(),
            allowed_caches: vec![AuthorizedCache {
                exact_identity_digest: identity.clone(),
                compatibility_group_digest: group.clone(),
            }],
            backend: "test-local".to_string(),
            expires_at_unix: 100,
        };
        let token = encode(
            &Header::new(Algorithm::EdDSA),
            &claims,
            &EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );

        let verified = verifier.verify(&headers, 99).unwrap();
        require_cache(&verified, &identity, &group, 99).unwrap();
        assert!(verifier.verify(&headers, 100).is_err());
        assert!(
            require_cache(
                &verified,
                &CacheDigest::parse("b".repeat(64)).unwrap(),
                &group,
                99,
            )
            .is_err()
        );
        assert!(
            GrantVerifier::new(TEST_PUBLIC_KEY, "other".to_string())
                .unwrap()
                .verify(&headers, 99)
                .is_err()
        );
    }

    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";
    const TEST_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";
}
