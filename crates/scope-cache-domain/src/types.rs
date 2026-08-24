use crate::{CacheDomainError, CachePolicy};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryId(String);

impl RepositoryId {
    pub fn parse(value: impl Into<String>) -> Result<Self, CacheDomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(CacheDomainError::MissingRepositoryId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RepositoryId {
    type Error = CacheDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RepositoryId> for String {
    fn from(value: RepositoryId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct CacheDigest(String);

impl CacheDigest {
    pub fn parse(value: impl Into<String>) -> Result<Self, CacheDomainError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CacheDomainError::InvalidDigest);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CacheDigest {
    type Error = CacheDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CacheDigest> for String {
    fn from(value: CacheDigest) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct UploadLeaseId(String);

impl UploadLeaseId {
    pub fn parse(value: impl Into<String>) -> Result<Self, CacheDomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(CacheDomainError::MissingUploadLeaseId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for UploadLeaseId {
    type Error = CacheDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UploadLeaseId> for String {
    fn from(value: UploadLeaseId) -> Self {
        value.0
    }
}

/// One immutable, repository-scoped, content-addressed object.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CacheObject {
    repository_id: RepositoryId,
    digest: CacheDigest,
    size_bytes: u64,
    created_at_unix: u64,
}

impl CacheObject {
    pub fn new(
        repository_id: RepositoryId,
        digest: CacheDigest,
        size_bytes: u64,
        created_at_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        policy.validate_object_size(size_bytes)?;
        Ok(Self {
            repository_id,
            digest,
            size_bytes,
            created_at_unix,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn digest(&self) -> &CacheDigest {
        &self.digest
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn created_at_unix(&self) -> u64 {
        self.created_at_unix
    }
}

/// The replaceable logical identity pointing to an immutable cache object.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CacheReference {
    repository_id: RepositoryId,
    identity_digest: CacheDigest,
    compatibility_group_digest: CacheDigest,
    object_digest: CacheDigest,
    updated_at_unix: u64,
    expires_at_unix: u64,
}

impl CacheReference {
    pub(crate) fn point_to(
        identity_digest: CacheDigest,
        compatibility_group_digest: CacheDigest,
        object: &CacheObject,
        now_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        Ok(Self {
            repository_id: object.repository_id.clone(),
            identity_digest,
            compatibility_group_digest,
            object_digest: object.digest.clone(),
            updated_at_unix: now_unix,
            expires_at_unix: policy.reference_expiry(now_unix)?,
        })
    }

    pub(crate) fn accessed_at(
        &self,
        now_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        if now_unix < self.updated_at_unix {
            return Err(CacheDomainError::ReferenceAccessBeforeLastUpdate);
        }
        Ok(Self {
            repository_id: self.repository_id.clone(),
            identity_digest: self.identity_digest.clone(),
            compatibility_group_digest: self.compatibility_group_digest.clone(),
            object_digest: self.object_digest.clone(),
            updated_at_unix: now_unix,
            expires_at_unix: policy.reference_expiry(now_unix)?,
        })
    }

    pub fn restore(
        repository_id: RepositoryId,
        identity_digest: CacheDigest,
        compatibility_group_digest: CacheDigest,
        object_digest: CacheDigest,
        updated_at_unix: u64,
        expires_at_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        if policy.reference_expiry(updated_at_unix)? != expires_at_unix {
            return Err(CacheDomainError::InvalidReferenceExpiry);
        }
        Ok(Self {
            repository_id,
            identity_digest,
            compatibility_group_digest,
            object_digest,
            updated_at_unix,
            expires_at_unix,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn identity_digest(&self) -> &CacheDigest {
        &self.identity_digest
    }

    pub fn compatibility_group_digest(&self) -> &CacheDigest {
        &self.compatibility_group_digest
    }

    pub fn object_digest(&self) -> &CacheDigest {
        &self.object_digest
    }

    pub fn updated_at_unix(&self) -> u64 {
        self.updated_at_unix
    }

    pub fn expires_at_unix(&self) -> u64 {
        self.expires_at_unix
    }

    pub fn is_expired_at(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_at_unix
    }
}

/// Exclusive permission to upload one exact object for one logical identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UploadLease {
    id: UploadLeaseId,
    repository_id: RepositoryId,
    identity_digest: CacheDigest,
    compatibility_group_digest: CacheDigest,
    object_digest: CacheDigest,
    size_bytes: u64,
    issued_at_unix: u64,
    expires_at_unix: u64,
}

impl UploadLease {
    pub(crate) fn issue(
        id: UploadLeaseId,
        identity_digest: CacheDigest,
        compatibility_group_digest: CacheDigest,
        object: &CacheObject,
        now_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        Ok(Self {
            id,
            repository_id: object.repository_id.clone(),
            identity_digest,
            compatibility_group_digest,
            object_digest: object.digest.clone(),
            size_bytes: object.size_bytes,
            issued_at_unix: now_unix,
            expires_at_unix: policy.upload_expiry(now_unix)?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: UploadLeaseId,
        repository_id: RepositoryId,
        identity_digest: CacheDigest,
        compatibility_group_digest: CacheDigest,
        object_digest: CacheDigest,
        size_bytes: u64,
        issued_at_unix: u64,
        expires_at_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        policy.validate_object_size(size_bytes)?;
        if policy.upload_expiry(issued_at_unix)? != expires_at_unix {
            return Err(CacheDomainError::InvalidUploadLeaseExpiry);
        }
        Ok(Self {
            id,
            repository_id,
            identity_digest,
            compatibility_group_digest,
            object_digest,
            size_bytes,
            issued_at_unix,
            expires_at_unix,
        })
    }

    pub fn id(&self) -> &UploadLeaseId {
        &self.id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn identity_digest(&self) -> &CacheDigest {
        &self.identity_digest
    }

    pub fn compatibility_group_digest(&self) -> &CacheDigest {
        &self.compatibility_group_digest
    }

    pub fn object_digest(&self) -> &CacheDigest {
        &self.object_digest
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn issued_at_unix(&self) -> u64 {
        self.issued_at_unix
    }

    pub fn expires_at_unix(&self) -> u64 {
        self.expires_at_unix
    }

    pub fn is_expired_at(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_at_unix
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeletionCandidate {
    repository_id: RepositoryId,
    object_digest: CacheDigest,
    eligible_after_unix: u64,
}

impl DeletionCandidate {
    pub(crate) fn after_reference_removal(
        reference: &CacheReference,
        now_unix: u64,
        policy: CachePolicy,
    ) -> Result<Self, CacheDomainError> {
        Ok(Self {
            repository_id: reference.repository_id.clone(),
            object_digest: reference.object_digest.clone(),
            eligible_after_unix: policy.deletion_eligible_at(now_unix)?,
        })
    }

    pub fn restore(
        repository_id: RepositoryId,
        object_digest: CacheDigest,
        eligible_after_unix: u64,
    ) -> Self {
        Self {
            repository_id,
            object_digest,
            eligible_after_unix,
        }
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn object_digest(&self) -> &CacheDigest {
        &self.object_digest
    }

    pub fn eligible_after_unix(&self) -> u64 {
        self.eligible_after_unix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_identifiers_cannot_be_bypassed_by_deserialization() {
        assert!(serde_json::from_str::<RepositoryId>(r#""  ""#).is_err());
        assert!(serde_json::from_str::<CacheDigest>(r#""ABC""#).is_err());
        assert!(serde_json::from_str::<UploadLeaseId>(r#""""#).is_err());
    }
}
