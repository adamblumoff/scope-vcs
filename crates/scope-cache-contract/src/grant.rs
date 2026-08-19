use scope_cache_domain::{CacheDigest, RepositoryId};
use serde::{Deserialize, Serialize};

/// Claims carried by a signed cache grant.
///
/// Token format, signing algorithm, key rotation, and verification are owned by
/// the service adapter. Keeping them out of the contract prevents wire DTOs from
/// becoming an authorization implementation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedCacheGrantClaims {
    pub repository_id: RepositoryId,
    pub allowed_identity_digests: Vec<CacheDigest>,
    pub backend: String,
    pub expires_at_unix: u64,
}

impl SignedCacheGrantClaims {
    pub fn allows_identity(&self, identity_digest: &CacheDigest, now_unix: u64) -> bool {
        now_unix < self.expires_at_unix
            && self
                .allowed_identity_digests
                .iter()
                .any(|allowed| allowed == identity_digest)
    }
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
            repository_id: RepositoryId::parse("repo-1").unwrap(),
            allowed_identity_digests: vec![digest('a')],
            backend: "railway-iad".to_string(),
            expires_at_unix: 100,
        };
        assert!(claims.allows_identity(&digest('a'), 99));
        assert!(!claims.allows_identity(&digest('b'), 99));
        assert!(!claims.allows_identity(&digest('a'), 100));

        let encoded = serde_json::to_value(claims).unwrap();
        assert_eq!(encoded["repository_id"], "repo-1");
        assert_eq!(encoded["backend"], "railway-iad");
    }

    #[test]
    fn malformed_identity_claims_are_rejected_during_deserialization() {
        let value = serde_json::json!({
            "repository_id": "repo-1",
            "allowed_identity_digests": ["not-a-digest"],
            "backend": "railway-iad",
            "expires_at_unix": 100,
        });
        assert!(serde_json::from_value::<SignedCacheGrantClaims>(value).is_err());
    }
}
