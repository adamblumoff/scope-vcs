use crate::{
    CacheDigest, CacheDomainError, CacheObject, CachePolicy, CacheReference, DeletionCandidate,
    UploadLease, UploadLeaseId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrepareUploadDecision {
    /// No upload is needed. Persist this reference to refresh its TTL or relink it.
    UseObject {
        reference: CacheReference,
        expected_reference_version: Option<u64>,
        superseded: Option<DeletionCandidate>,
    },
    Upload {
        lease: UploadLease,
    },
}

#[derive(Clone, Debug)]
pub struct PrepareUpload<'a> {
    pub identity_digest: CacheDigest,
    pub object: &'a CacheObject,
    pub object_already_stored: bool,
    pub current_reference: Option<&'a CacheReference>,
    pub repository_storage_bytes: u64,
    pub lease_id: UploadLeaseId,
    pub now_unix: u64,
}

pub fn prepare_upload(
    policy: CachePolicy,
    request: PrepareUpload<'_>,
) -> Result<PrepareUploadDecision, CacheDomainError> {
    validate_reference_scope(
        request.current_reference,
        request.object,
        &request.identity_digest,
    )?;

    let already_available = request.object_already_stored
        || request
            .current_reference
            .is_some_and(|reference| reference.object_digest() == request.object.digest());
    if already_available {
        let superseded = request
            .current_reference
            .filter(|reference| reference.object_digest() != request.object.digest())
            .map(|reference| {
                DeletionCandidate::after_reference_removal(reference, request.now_unix, policy)
            })
            .transpose()?;
        return Ok(PrepareUploadDecision::UseObject {
            reference: CacheReference::point_to(
                request.identity_digest,
                request.object,
                request.current_reference,
                request.now_unix,
                policy,
            )?,
            expected_reference_version: request.current_reference.map(CacheReference::version),
            superseded,
        });
    }

    policy.validate_repository_growth(
        request.repository_storage_bytes,
        request.object.size_bytes(),
    )?;
    Ok(PrepareUploadDecision::Upload {
        lease: UploadLease::issue(
            request.lease_id,
            request.identity_digest,
            request.object,
            request.current_reference,
            request.now_unix,
            policy,
        )?,
    })
}

/// Refresh a successful restore without changing the logical reference revision.
///
/// Access does not repoint the identity, so an upload lease prepared against the
/// same version remains valid. The access timestamp and policy-owned TTL advance
/// together and can be persisted as one atomic update.
pub fn access_reference(
    policy: CachePolicy,
    reference: &CacheReference,
    now_unix: u64,
) -> Result<CacheReference, CacheDomainError> {
    reference.accessed_at(now_unix, policy)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitUploadDecision {
    Committed {
        reference: CacheReference,
        superseded: Option<DeletionCandidate>,
    },
    AlreadyCommitted {
        reference: CacheReference,
    },
}

pub fn commit_upload(
    policy: CachePolicy,
    lease: &UploadLease,
    uploaded_object: &CacheObject,
    current_reference: Option<&CacheReference>,
    now_unix: u64,
) -> Result<CommitUploadDecision, CacheDomainError> {
    validate_reference_scope(current_reference, uploaded_object, lease.identity_digest())?;
    if lease.repository_id() != uploaded_object.repository_id()
        || lease.object_digest() != uploaded_object.digest()
        || lease.size_bytes() != uploaded_object.size_bytes()
    {
        return Err(CacheDomainError::UploadLeaseMismatch);
    }

    if let Some(reference) = current_reference.filter(|reference| {
        reference.object_digest() == uploaded_object.digest()
            && reference.identity_digest() == lease.identity_digest()
    }) {
        return Ok(CommitUploadDecision::AlreadyCommitted {
            reference: reference.clone(),
        });
    }
    if current_reference.map(CacheReference::version) != lease.expected_reference_version() {
        return Err(CacheDomainError::StaleUploadLease);
    }
    if lease.is_expired_at(now_unix) {
        return Err(CacheDomainError::UploadLeaseExpired);
    }

    let reference = CacheReference::point_to(
        lease.identity_digest().clone(),
        uploaded_object,
        current_reference,
        now_unix,
        policy,
    )?;
    let superseded = current_reference
        .map(|previous| DeletionCandidate::after_reference_removal(previous, now_unix, policy))
        .transpose()?;
    Ok(CommitUploadDecision::Committed {
        reference,
        superseded,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionCause {
    Expired,
    RepositoryBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionReason {
    ActiveReference,
    GracePeriod,
    Referenced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvictionDecision {
    Retain {
        reason: RetentionReason,
    },
    RemoveReference {
        cause: EvictionCause,
        deletion: DeletionCandidate,
    },
    DeleteObject {
        repository_id: crate::RepositoryId,
        object_digest: CacheDigest,
    },
}

/// Decide whether a selected logical reference should be removed.
///
/// When a repository is over budget, the persistence adapter selects the least
/// recently used reference and passes it here. This function owns the actual rule.
pub fn decide_reference_eviction(
    policy: CachePolicy,
    reference: &CacheReference,
    repository_storage_bytes: u64,
    now_unix: u64,
) -> Result<EvictionDecision, CacheDomainError> {
    let cause = if reference.is_expired_at(now_unix) {
        Some(EvictionCause::Expired)
    } else if repository_storage_bytes > policy.max_repository_bytes() {
        Some(EvictionCause::RepositoryBudget)
    } else {
        None
    };

    match cause {
        Some(cause) => Ok(EvictionDecision::RemoveReference {
            cause,
            deletion: DeletionCandidate::after_reference_removal(reference, now_unix, policy)?,
        }),
        None => Ok(EvictionDecision::Retain {
            reason: RetentionReason::ActiveReference,
        }),
    }
}

pub fn decide_object_deletion(
    candidate: &DeletionCandidate,
    live_reference_count: u64,
    now_unix: u64,
) -> EvictionDecision {
    if live_reference_count > 0 {
        EvictionDecision::Retain {
            reason: RetentionReason::Referenced,
        }
    } else if now_unix < candidate.eligible_after_unix() {
        EvictionDecision::Retain {
            reason: RetentionReason::GracePeriod,
        }
    } else {
        EvictionDecision::DeleteObject {
            repository_id: candidate.repository_id().clone(),
            object_digest: candidate.object_digest().clone(),
        }
    }
}

fn validate_reference_scope(
    reference: Option<&CacheReference>,
    object: &CacheObject,
    identity_digest: &CacheDigest,
) -> Result<(), CacheDomainError> {
    if reference.is_some_and(|reference| {
        reference.repository_id() != object.repository_id()
            || reference.identity_digest() != identity_digest
    }) {
        return Err(CacheDomainError::ReferenceScopeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CACHE_REFERENCE_TTL_SECONDS, DELETION_GRACE_SECONDS, MAX_REPOSITORY_CACHE_BYTES,
        RepositoryId, UPLOAD_LEASE_SECONDS,
    };

    fn digest(value: char) -> CacheDigest {
        CacheDigest::parse(value.to_string().repeat(64)).unwrap()
    }

    fn cache_object(
        repository: &str,
        digest_value: char,
        size_bytes: u64,
        now: u64,
    ) -> CacheObject {
        CacheObject::new(
            RepositoryId::parse(repository).unwrap(),
            digest(digest_value),
            size_bytes,
            now,
            CachePolicy,
        )
        .unwrap()
    }

    fn upload(
        object: &CacheObject,
        identity_digest: CacheDigest,
        current_reference: Option<&CacheReference>,
        object_already_stored: bool,
        repository_storage_bytes: u64,
        now: u64,
    ) -> Result<PrepareUploadDecision, CacheDomainError> {
        prepare_upload(
            CachePolicy,
            PrepareUpload {
                identity_digest,
                object,
                object_already_stored,
                current_reference,
                repository_storage_bytes,
                lease_id: UploadLeaseId::parse("lease-1").unwrap(),
                now_unix: now,
            },
        )
    }

    #[test]
    fn existing_content_is_referenced_without_an_upload_and_refreshes_ttl() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject {
            reference,
            expected_reference_version,
            superseded,
        } = upload(&object, digest('b'), None, true, 100, 10).unwrap()
        else {
            panic!("expected existing object to be reused");
        };
        assert_eq!(reference.object_digest(), object.digest());
        assert_eq!(reference.version(), 1);
        assert_eq!(expected_reference_version, None);
        assert_eq!(superseded, None);
        assert_eq!(reference.updated_at_unix(), 10);
        assert_eq!(
            reference.expires_at_unix(),
            10 + CACHE_REFERENCE_TTL_SECONDS
        );
    }

    #[test]
    fn access_refreshes_ttl_without_changing_identity_object_or_version() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject { reference, .. } =
            upload(&object, digest('b'), None, true, 100, 10).unwrap()
        else {
            unreachable!();
        };

        let refreshed = access_reference(CachePolicy, &reference, 20).unwrap();
        assert_eq!(refreshed.repository_id(), reference.repository_id());
        assert_eq!(refreshed.identity_digest(), reference.identity_digest());
        assert_eq!(refreshed.object_digest(), reference.object_digest());
        assert_eq!(refreshed.version(), reference.version());
        assert_eq!(refreshed.updated_at_unix(), 20);
        assert_eq!(
            refreshed.expires_at_unix(),
            20 + CACHE_REFERENCE_TTL_SECONDS
        );
        assert_eq!(
            access_reference(CachePolicy, &refreshed, 19),
            Err(CacheDomainError::ReferenceAccessBeforeLastUpdate)
        );
    }

    #[test]
    fn reusing_a_different_stored_object_marks_the_old_object_for_deletion() {
        let first = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject {
            reference: first_reference,
            ..
        } = upload(&first, digest('b'), None, true, 100, 10).unwrap()
        else {
            unreachable!();
        };
        let replacement = cache_object("repo-1", 'c', 100, 20);
        let PrepareUploadDecision::UseObject {
            reference,
            expected_reference_version,
            superseded,
        } = upload(
            &replacement,
            digest('b'),
            Some(&first_reference),
            true,
            200,
            20,
        )
        .unwrap()
        else {
            unreachable!();
        };

        assert_eq!(reference.version(), 2);
        assert_eq!(reference.object_digest(), replacement.digest());
        assert_eq!(expected_reference_version, Some(1));
        let superseded = superseded.expect("old object must be considered for deletion");
        assert_eq!(superseded.object_digest(), first.digest());
        assert_eq!(
            superseded.eligible_after_unix(),
            20 + DELETION_GRACE_SECONDS
        );
    }

    #[test]
    fn a_new_object_gets_a_short_lease_and_must_fit_the_repository_budget() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::Upload { lease } =
            upload(&object, digest('b'), None, false, 200, 10).unwrap()
        else {
            panic!("expected upload lease");
        };
        assert_eq!(lease.expires_at_unix(), 10 + UPLOAD_LEASE_SECONDS);

        assert!(matches!(
            upload(
                &object,
                digest('b'),
                None,
                false,
                MAX_REPOSITORY_CACHE_BYTES,
                10,
            ),
            Err(CacheDomainError::RepositoryBudgetExceeded { .. })
        ));
    }

    #[test]
    fn commit_is_scoped_exact_idempotent_and_rejects_expired_leases() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::Upload { lease } =
            upload(&object, digest('b'), None, false, 0, 10).unwrap()
        else {
            unreachable!();
        };
        let CommitUploadDecision::Committed {
            reference,
            superseded,
        } = commit_upload(CachePolicy, &lease, &object, None, 20).unwrap()
        else {
            unreachable!();
        };
        assert!(superseded.is_none());

        assert!(matches!(
            commit_upload(
                CachePolicy,
                &lease,
                &object,
                Some(&reference),
                lease.expires_at_unix(),
            ),
            Ok(CommitUploadDecision::AlreadyCommitted { .. })
        ));

        let other_object = cache_object("repo-2", 'a', 100, 1);
        assert_eq!(
            commit_upload(CachePolicy, &lease, &other_object, None, 20),
            Err(CacheDomainError::UploadLeaseMismatch)
        );

        let expired_object = cache_object("repo-1", 'c', 100, 1);
        let PrepareUploadDecision::Upload {
            lease: expired_lease,
        } = upload(&expired_object, digest('d'), None, false, 0, 10).unwrap()
        else {
            unreachable!();
        };
        assert_eq!(
            commit_upload(
                CachePolicy,
                &expired_lease,
                &expired_object,
                None,
                expired_lease.expires_at_unix(),
            ),
            Err(CacheDomainError::UploadLeaseExpired)
        );
    }

    #[test]
    fn commit_rejects_a_lease_when_another_writer_advanced_the_reference() {
        let original = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject {
            reference: original_reference,
            ..
        } = upload(&original, digest('b'), None, true, 100, 10).unwrap()
        else {
            unreachable!();
        };
        let replacement = cache_object("repo-1", 'c', 100, 20);
        let PrepareUploadDecision::Upload { lease } = upload(
            &replacement,
            digest('b'),
            Some(&original_reference),
            false,
            100,
            20,
        )
        .unwrap() else {
            unreachable!();
        };
        assert_eq!(lease.expected_reference_version(), Some(1));

        let winner = cache_object("repo-1", 'd', 100, 21);
        let PrepareUploadDecision::UseObject {
            reference: advanced_reference,
            ..
        } = upload(
            &winner,
            digest('b'),
            Some(&original_reference),
            true,
            200,
            21,
        )
        .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(advanced_reference.version(), 2);
        assert_eq!(
            commit_upload(
                CachePolicy,
                &lease,
                &replacement,
                Some(&advanced_reference),
                22,
            ),
            Err(CacheDomainError::StaleUploadLease)
        );
    }

    #[test]
    fn eviction_honors_ttl_budget_references_and_deletion_grace() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject { reference, .. } =
            upload(&object, digest('b'), None, true, 100, 10).unwrap()
        else {
            unreachable!();
        };
        assert_eq!(
            decide_reference_eviction(CachePolicy, &reference, 100, 20).unwrap(),
            EvictionDecision::Retain {
                reason: RetentionReason::ActiveReference,
            }
        );

        let EvictionDecision::RemoveReference { cause, deletion } =
            decide_reference_eviction(CachePolicy, &reference, 100, reference.expires_at_unix())
                .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(cause, EvictionCause::Expired);
        assert_eq!(
            deletion.eligible_after_unix(),
            reference.expires_at_unix() + DELETION_GRACE_SECONDS
        );
        assert_eq!(
            decide_object_deletion(&deletion, 1, deletion.eligible_after_unix()),
            EvictionDecision::Retain {
                reason: RetentionReason::Referenced,
            }
        );
        assert_eq!(
            decide_object_deletion(&deletion, 0, deletion.eligible_after_unix() - 1),
            EvictionDecision::Retain {
                reason: RetentionReason::GracePeriod,
            }
        );
        assert!(matches!(
            decide_object_deletion(&deletion, 0, deletion.eligible_after_unix()),
            EvictionDecision::DeleteObject { .. }
        ));

        assert!(matches!(
            decide_reference_eviction(CachePolicy, &reference, MAX_REPOSITORY_CACHE_BYTES + 1, 20,),
            Ok(EvictionDecision::RemoveReference {
                cause: EvictionCause::RepositoryBudget,
                ..
            })
        ));
    }

    #[test]
    fn a_reference_cannot_cross_repository_or_identity_boundaries() {
        let object = cache_object("repo-1", 'a', 100, 1);
        let PrepareUploadDecision::UseObject { reference, .. } =
            upload(&object, digest('b'), None, true, 100, 10).unwrap()
        else {
            unreachable!();
        };
        let other_repo_object = cache_object("repo-2", 'c', 100, 1);
        assert_eq!(
            upload(
                &other_repo_object,
                digest('b'),
                Some(&reference),
                false,
                0,
                20,
            ),
            Err(CacheDomainError::ReferenceScopeMismatch)
        );
        assert_eq!(
            upload(&object, digest('d'), Some(&reference), false, 0, 20,),
            Err(CacheDomainError::ReferenceScopeMismatch)
        );
    }
}
