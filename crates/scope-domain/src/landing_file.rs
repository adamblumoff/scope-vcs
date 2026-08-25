use crate::{
    content::{SourceBlob, is_supported_git_file_mode},
    content_ref::ContentRef,
    error::DomainError,
};
use sha2::{Digest, Sha256};

pub const REPOSITORY_LANDING_FILE_PATH: &str = "/README.html";
pub const MAX_REPOSITORY_LANDING_FILE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryLandingFile {
    pub oid: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub git_file_mode: String,
    pub content_bytes: Vec<u8>,
}

impl RepositoryLandingFile {
    pub fn from_source_blob(
        blob: &SourceBlob,
        content_bytes: Vec<u8>,
    ) -> Result<Self, DomainError> {
        if !source_identity_matches(blob) {
            return Err(DomainError::invariant_violation(
                "repository landing file has an invalid source identity",
            ));
        }
        if blob.size_bytes != content_bytes.len() as u64 {
            return Err(DomainError::invariant_violation(
                "repository landing file size does not match its source blob",
            ));
        }

        let landing_file = Self {
            oid: blob.git_oid.clone(),
            sha256: hex::encode(Sha256::digest(&content_bytes)),
            size_bytes: blob.size_bytes,
            git_file_mode: blob.git_file_mode.clone(),
            content_bytes,
        };
        landing_file.validate_integrity()?;
        if matches!(blob.content_ref, ContentRef::BlobSha256(_))
            && landing_file.sha256 != blob.sha256
        {
            return Err(DomainError::invariant_violation(
                "repository landing file digest does not match its source blob",
            ));
        }
        Ok(landing_file)
    }

    pub fn validate_integrity(&self) -> Result<(), DomainError> {
        if self.oid.is_empty() || self.oid.len() > 128 {
            return Err(DomainError::invariant_violation(
                "repository landing file has an invalid Git object ID",
            ));
        }
        if !is_supported_git_file_mode(&self.git_file_mode) {
            return Err(DomainError::invariant_violation(
                "repository landing file has an unsupported Git mode",
            ));
        }
        if self.content_bytes.len() > MAX_REPOSITORY_LANDING_FILE_BYTES {
            return Err(DomainError::invariant_violation(
                "repository landing file exceeds the rendered-text limit",
            ));
        }
        if self.size_bytes != self.content_bytes.len() as u64 {
            return Err(DomainError::invariant_violation(
                "repository landing file size does not match its content",
            ));
        }
        if self.sha256 != hex::encode(Sha256::digest(&self.content_bytes)) {
            return Err(DomainError::invariant_violation(
                "repository landing file digest does not match its content",
            ));
        }
        Ok(())
    }

    pub fn verify_source(&self, blob: &SourceBlob) -> Result<(), DomainError> {
        self.validate_integrity()?;
        let valid = source_identity_matches(blob)
            && self.oid == blob.git_oid
            && self.git_file_mode == blob.git_file_mode
            && self.size_bytes == blob.size_bytes
            && (!matches!(blob.content_ref, ContentRef::BlobSha256(_))
                || self.sha256 == blob.sha256);
        if valid {
            Ok(())
        } else {
            Err(DomainError::invariant_violation(
                "repository landing file does not match projected metadata",
            ))
        }
    }
}

fn source_identity_matches(blob: &SourceBlob) -> bool {
    match &blob.content_ref {
        ContentRef::GitBlob { git_oid } => git_oid == &blob.git_oid,
        ContentRef::BlobSha256(sha256) => sha256 == &blob.sha256,
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoryLandingFileMutation {
    Unchanged,
    Upsert(RepositoryLandingFile),
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::DEFAULT_GIT_FILE_MODE;

    fn git_blob(bytes: &[u8]) -> SourceBlob {
        SourceBlob {
            content_ref: ContentRef::git_blob("abc123"),
            sha256: "abc123".to_string(),
            git_oid: "abc123".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: bytes.len() as u64,
        }
    }

    #[test]
    fn landing_file_is_bounded_and_verifies_projected_identity() {
        let bytes = b"<h1>Scope</h1>".to_vec();
        let blob = git_blob(&bytes);
        let landing = RepositoryLandingFile::from_source_blob(&blob, bytes).unwrap();

        landing.verify_source(&blob).unwrap();
        assert_eq!(landing.oid, "abc123");
        assert_eq!(landing.sha256.len(), 64);
    }

    #[test]
    fn landing_file_rejects_oversized_content() {
        let bytes = vec![0; MAX_REPOSITORY_LANDING_FILE_BYTES + 1];
        let blob = git_blob(&bytes);

        assert!(RepositoryLandingFile::from_source_blob(&blob, bytes).is_err());
    }

    #[test]
    fn landing_file_detects_corrupt_persisted_bytes() {
        let bytes = b"<h1>Scope</h1>".to_vec();
        let blob = git_blob(&bytes);
        let mut landing = RepositoryLandingFile::from_source_blob(&blob, bytes).unwrap();
        landing.content_bytes.push(b'!');

        assert!(landing.verify_source(&blob).is_err());
    }

    #[test]
    fn landing_file_accepts_verified_direct_blob_sources_for_backfill() {
        let bytes = b"<h1>seeded</h1>".to_vec();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let blob = SourceBlob {
            content_ref: ContentRef::blob_sha256(&sha256),
            sha256,
            git_oid: "abc123".to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: bytes.len() as u64,
        };

        let landing = RepositoryLandingFile::from_source_blob(&blob, bytes).unwrap();
        landing.verify_source(&blob).unwrap();
    }
}
