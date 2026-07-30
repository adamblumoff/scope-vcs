use super::runner::validate_sha256_hash;
use crate::{
    content_ref::ContentRef, error::DomainError, projection::ProjectionViewKey, store::SourceBlob,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunTrigger {
    Manual,
    PushMain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunSource {
    EphemeralGitBundle {
        object: SourceBlob,
    },
    AcceptedRevision {
        change_version: u64,
        manifest: SourceBlob,
        snapshot: SourceBlob,
        audience: ProjectionViewKey,
    },
}

impl RunSource {
    pub fn ephemeral_git_bundle(object: SourceBlob) -> Result<Self, DomainError> {
        validate_source_blob(&object, "run source bundle")?;
        if !matches!(object.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "ephemeral run source must be a Git bundle",
            ));
        }
        Ok(Self::EphemeralGitBundle { object })
    }

    pub fn accepted_revision(
        change_version: u64,
        manifest: SourceBlob,
        snapshot: SourceBlob,
        audience: ProjectionViewKey,
    ) -> Result<Self, DomainError> {
        if change_version == 0 {
            return Err(DomainError::invalid_input(
                "accepted run source change version must be positive",
            ));
        }
        validate_source_blob(&manifest, "run source manifest")?;
        if !matches!(manifest.content_ref, ContentRef::GitManifestSha256(_)) {
            return Err(DomainError::invalid_input(
                "accepted run source must use a Git manifest",
            ));
        }
        validate_source_blob(&snapshot, "accepted run source snapshot")?;
        if !matches!(snapshot.content_ref, ContentRef::GitBundleSha256(_)) {
            return Err(DomainError::invalid_input(
                "accepted run source snapshot must be a Git bundle",
            ));
        }
        if snapshot.git_oid != manifest.git_oid {
            return Err(DomainError::invalid_input(
                "accepted run source snapshot and manifest heads do not match",
            ));
        }
        Ok(Self::AcceptedRevision {
            change_version,
            manifest,
            snapshot,
            audience,
        })
    }

    pub fn snapshot(&self) -> &SourceBlob {
        match self {
            Self::EphemeralGitBundle { object } => object,
            Self::AcceptedRevision { snapshot, .. } => snapshot,
        }
    }

    pub fn retained_objects(&self) -> Vec<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => vec![object],
            Self::AcceptedRevision {
                manifest, snapshot, ..
            } => vec![manifest, snapshot],
        }
    }

    pub fn digest(&self) -> &str {
        &self.snapshot().sha256
    }

    pub fn git_oid(&self) -> &str {
        &self.snapshot().git_oid
    }

    pub fn is_private_only(&self) -> bool {
        matches!(
            self,
            Self::EphemeralGitBundle { .. }
                | Self::AcceptedRevision {
                    audience: ProjectionViewKey::Private,
                    ..
                }
        )
    }
}

fn validate_source_blob(blob: &SourceBlob, label: &str) -> Result<(), DomainError> {
    validate_sha256_hash(&format!("{label} digest"), &blob.sha256)?;
    if blob.content_ref.sha256() != Some(blob.sha256.as_str()) {
        return Err(DomainError::invalid_input(format!(
            "{label} content reference does not match its digest"
        )));
    }
    validate_git_oid(&format!("{label} Git OID"), &blob.git_oid)
}

fn validate_git_oid(label: &str, git_oid: &str) -> Result<(), DomainError> {
    if git_oid.len() != 40 || !git_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-1 hex digest"
        )));
    }
    Ok(())
}
