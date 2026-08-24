use scope_cache_domain::{CacheDigest, RepositoryId};
use serde::{Deserialize, Serialize};

/// Claims carried by a signed cache grant.
///
/// Token format, signing algorithm, key rotation, and verification are owned by
/// the service adapter. Keeping them out of the contract prevents wire DTOs from
/// becoming an authorization implementation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedCacheGrantClaims {
    pub attempt_id: String,
    pub repository_id: RepositoryId,
    pub allowed_caches: Vec<AuthorizedCache>,
    pub backend: String,
    pub expires_at_unix: u64,
}

impl SignedCacheGrantClaims {
    pub fn allows_cache(
        &self,
        exact_identity_digest: &CacheDigest,
        compatibility_group_digest: &CacheDigest,
        now_unix: u64,
    ) -> bool {
        now_unix < self.expires_at_unix
            && self.allowed_caches.iter().any(|allowed| {
                &allowed.exact_identity_digest == exact_identity_digest
                    && &allowed.compatibility_group_digest == compatibility_group_digest
            })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthorizedCache {
    pub exact_identity_digest: CacheDigest,
    pub compatibility_group_digest: CacheDigest,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(value: char) -> CacheDigest {
        CacheDigest::parse(value.to_string().repeat(64)).unwrap()
    }

    #[test]
    fn grant_is_repository_backend_and_identity_scoped() {
        let claims = SignedCacheGrantClaims {
            attempt_id: "attempt-1".to_string(),
            repository_id: RepositoryId::parse("repo-1").unwrap(),
            allowed_caches: vec![AuthorizedCache {
                exact_identity_digest: digest('a'),
                compatibility_group_digest: digest('b'),
            }],
            backend: "railway-iad".to_string(),
            expires_at_unix: 100,
        };
        assert!(claims.allows_cache(&digest('a'), &digest('b'), 99));
        assert!(!claims.allows_cache(&digest('a'), &digest('c'), 99));
        assert!(!claims.allows_cache(&digest('a'), &digest('b'), 100));

        let encoded = serde_json::to_value(claims).unwrap();
        assert_eq!(encoded["repository_id"], "repo-1");
        assert_eq!(encoded["attempt_id"], "attempt-1");
        assert_eq!(encoded["backend"], "railway-iad");
    }

    #[test]
    fn malformed_identity_claims_are_rejected_during_deserialization() {
        let value = serde_json::json!({
            "attempt_id": "attempt-1",
            "repository_id": "repo-1",
            "allowed_caches": [{
                "exact_identity_digest": "not-a-digest",
                "compatibility_group_digest": "b".repeat(64),
            }],
            "backend": "railway-iad",
            "expires_at_unix": 100,
        });
        assert!(serde_json::from_value::<SignedCacheGrantClaims>(value).is_err());
    }
}
