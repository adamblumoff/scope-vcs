use crate::CacheDomainError;

pub const GIB: u64 = 1024 * 1024 * 1024;
pub const MAX_CACHE_OBJECT_BYTES: u64 = GIB;
pub const MAX_REPOSITORY_CACHE_BYTES: u64 = 5 * GIB;
pub const CACHE_REFERENCE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const UPLOAD_LEASE_SECONDS: u64 = 30 * 60;
pub const DELETION_GRACE_SECONDS: u64 = 60 * 60;

/// The cache service's fixed safety and retention policy.
///
/// Keeping the values fixed makes every adapter enforce the same limits. A future
/// product decision can make policy configurable without weakening today's domain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CachePolicy;

impl CachePolicy {
    pub const fn max_object_bytes(self) -> u64 {
        MAX_CACHE_OBJECT_BYTES
    }

    pub const fn max_repository_bytes(self) -> u64 {
        MAX_REPOSITORY_CACHE_BYTES
    }

    pub const fn reference_ttl_seconds(self) -> u64 {
        CACHE_REFERENCE_TTL_SECONDS
    }

    pub const fn upload_lease_seconds(self) -> u64 {
        UPLOAD_LEASE_SECONDS
    }

    pub const fn deletion_grace_seconds(self) -> u64 {
        DELETION_GRACE_SECONDS
    }

    pub fn validate_object_size(self, size_bytes: u64) -> Result<(), CacheDomainError> {
        if size_bytes == 0 {
            return Err(CacheDomainError::EmptyObject);
        }
        if size_bytes > self.max_object_bytes() {
            return Err(CacheDomainError::ObjectTooLarge {
                actual_bytes: size_bytes,
                maximum_bytes: self.max_object_bytes(),
            });
        }
        Ok(())
    }

    pub fn validate_repository_growth(
        self,
        stored_bytes: u64,
        additional_bytes: u64,
    ) -> Result<(), CacheDomainError> {
        let requested_bytes = stored_bytes
            .checked_add(additional_bytes)
            .ok_or(CacheDomainError::ByteCountOverflow)?;
        if requested_bytes > self.max_repository_bytes() {
            return Err(CacheDomainError::RepositoryBudgetExceeded {
                requested_bytes,
                maximum_bytes: self.max_repository_bytes(),
            });
        }
        Ok(())
    }

    pub(crate) fn reference_expiry(self, now_unix: u64) -> Result<u64, CacheDomainError> {
        now_unix
            .checked_add(self.reference_ttl_seconds())
            .ok_or(CacheDomainError::TimestampOverflow)
    }

    pub(crate) fn upload_expiry(self, now_unix: u64) -> Result<u64, CacheDomainError> {
        now_unix
            .checked_add(self.upload_lease_seconds())
            .ok_or(CacheDomainError::TimestampOverflow)
    }

    pub(crate) fn deletion_eligible_at(self, now_unix: u64) -> Result<u64, CacheDomainError> {
        now_unix
            .checked_add(self.deletion_grace_seconds())
            .ok_or(CacheDomainError::TimestampOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_limits_accept_the_boundary_and_reject_growth_past_it() {
        let policy = CachePolicy;
        assert_eq!(policy.validate_object_size(MAX_CACHE_OBJECT_BYTES), Ok(()));
        assert!(matches!(
            policy.validate_object_size(MAX_CACHE_OBJECT_BYTES + 1),
            Err(CacheDomainError::ObjectTooLarge { .. })
        ));
        assert_eq!(
            policy.validate_repository_growth(
                MAX_REPOSITORY_CACHE_BYTES - MAX_CACHE_OBJECT_BYTES,
                MAX_CACHE_OBJECT_BYTES,
            ),
            Ok(())
        );
        assert!(matches!(
            policy.validate_repository_growth(MAX_REPOSITORY_CACHE_BYTES, 1),
            Err(CacheDomainError::RepositoryBudgetExceeded { .. })
        ));
    }
}
