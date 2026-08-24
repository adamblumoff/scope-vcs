use super::workflow::{WorkflowError, WorkflowPath};
use crate::{
    content_ref::ContentRef,
    store::{SourceBlob, is_supported_git_file_mode},
};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::BTreeSet;
use thiserror::Error;

pub const MAX_REPOSITORY_WORKFLOW_FILES: usize = 64;
pub const MAX_WORKFLOW_DEFINITION_BYTES: usize = 64 * 1024;
pub const MAX_REPOSITORY_WORKFLOW_CONFIGURATION_ERROR_BYTES: usize = 4 * 1024;

#[derive(Debug, Error)]
pub enum RepositoryWorkflowCatalogError {
    #[error("repository workflow catalog requires a repository id")]
    MissingRepositoryId,
    #[error("repository workflow catalog head must be a SHA-1 hex digest")]
    InvalidSourceHead,
    #[error("repository workflow catalog change version must be greater than zero")]
    InvalidSourceChangeVersion,
    #[error("repository workflow catalog rejection requires a configuration error")]
    MissingConfigurationError,
    #[error(
        "repository workflow catalog configuration error exceeds {MAX_REPOSITORY_WORKFLOW_CONFIGURATION_ERROR_BYTES} bytes"
    )]
    ConfigurationErrorTooLarge,
    #[error("repository workflow catalog does not match the accepted repository state")]
    SourceMismatch,
    #[error("repository contains more than {MAX_REPOSITORY_WORKFLOW_FILES} workflow definitions")]
    TooManyFiles,
    #[error("workflow path {0} appears more than once in the repository catalog")]
    DuplicatePath(String),
    #[error(transparent)]
    InvalidPath(#[from] WorkflowError),
    #[error("workflow {path} has an invalid Git object ID")]
    InvalidGitObjectId { path: String },
    #[error("workflow {path} has unsupported Git file mode {mode}")]
    UnsupportedGitFileMode { path: String, mode: String },
    #[error("workflow {path} exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes")]
    DefinitionTooLarge { path: String },
    #[error("workflow {path} size does not match its content")]
    SizeMismatch { path: String },
    #[error("workflow {path} content does not match its Git object ID")]
    GitObjectMismatch { path: String },
    #[error("workflow {path} does not match its source blob")]
    SourceBlobMismatch { path: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryWorkflowFile {
    path: WorkflowPath,
    oid: String,
    size_bytes: u64,
    git_file_mode: String,
    content_bytes: Vec<u8>,
}

impl RepositoryWorkflowFile {
    pub fn from_source_blob(
        path: impl Into<String>,
        blob: &SourceBlob,
        content_bytes: Vec<u8>,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        let path = path.into();
        if !source_blob_identity_matches(blob) {
            return Err(RepositoryWorkflowCatalogError::SourceBlobMismatch { path });
        }
        let file = Self::new(
            path,
            blob.git_oid.clone(),
            blob.size_bytes,
            blob.git_file_mode.clone(),
            content_bytes,
        )?;
        file.verify_source(blob)?;
        Ok(file)
    }

    pub fn new(
        path: impl Into<String>,
        oid: impl Into<String>,
        size_bytes: u64,
        git_file_mode: impl Into<String>,
        content_bytes: Vec<u8>,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        let path = WorkflowPath::parse(path.into())?;
        let file = Self {
            path,
            oid: oid.into().to_ascii_lowercase(),
            size_bytes,
            git_file_mode: git_file_mode.into(),
            content_bytes,
        };
        file.validate_integrity()?;
        Ok(file)
    }

    pub fn from_content(
        path: impl Into<String>,
        git_file_mode: impl Into<String>,
        content_bytes: Vec<u8>,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        let oid = git_blob_oid(&content_bytes);
        Self::new(
            path,
            oid,
            content_bytes.len() as u64,
            git_file_mode,
            content_bytes,
        )
    }

    pub fn path(&self) -> &WorkflowPath {
        &self.path
    }

    pub fn oid(&self) -> &str {
        &self.oid
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn git_file_mode(&self) -> &str {
        &self.git_file_mode
    }

    pub fn content_bytes(&self) -> &[u8] {
        &self.content_bytes
    }

    pub fn validate_integrity(&self) -> Result<(), RepositoryWorkflowCatalogError> {
        let path = self.path.as_str().to_string();
        if !is_sha1_hex(&self.oid) {
            return Err(RepositoryWorkflowCatalogError::InvalidGitObjectId { path });
        }
        if !is_supported_git_file_mode(&self.git_file_mode) {
            return Err(RepositoryWorkflowCatalogError::UnsupportedGitFileMode {
                path,
                mode: self.git_file_mode.clone(),
            });
        }
        if self.content_bytes.len() > MAX_WORKFLOW_DEFINITION_BYTES {
            return Err(RepositoryWorkflowCatalogError::DefinitionTooLarge { path });
        }
        if self.size_bytes != self.content_bytes.len() as u64 {
            return Err(RepositoryWorkflowCatalogError::SizeMismatch { path });
        }
        if self.oid != git_blob_oid(&self.content_bytes) {
            return Err(RepositoryWorkflowCatalogError::GitObjectMismatch { path });
        }
        Ok(())
    }

    pub fn verify_source(&self, blob: &SourceBlob) -> Result<(), RepositoryWorkflowCatalogError> {
        self.validate_integrity()?;
        let digest_matches = !matches!(blob.content_ref, ContentRef::BlobSha256(_))
            || blob.sha256 == hex::encode(Sha256::digest(&self.content_bytes));
        if source_blob_identity_matches(blob)
            && self.oid == blob.git_oid.to_ascii_lowercase()
            && self.git_file_mode == blob.git_file_mode
            && self.size_bytes == blob.size_bytes
            && digest_matches
        {
            Ok(())
        } else {
            Err(RepositoryWorkflowCatalogError::SourceBlobMismatch {
                path: self.path.as_str().to_string(),
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RepositoryWorkflowCatalogState {
    Captured(Vec<RepositoryWorkflowFile>),
    Rejected(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryWorkflowCatalog {
    repository_id: String,
    source_head_oid: String,
    source_change_version: u64,
    state: RepositoryWorkflowCatalogState,
}

impl RepositoryWorkflowCatalog {
    pub fn captured(
        repository_id: impl Into<String>,
        source_head_oid: impl Into<String>,
        source_change_version: u64,
        mut files: Vec<RepositoryWorkflowFile>,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        let repository_id = repository_id.into();
        let source_head_oid = source_head_oid.into().to_ascii_lowercase();
        validate_identity(&repository_id, &source_head_oid, source_change_version)?;
        if files.len() > MAX_REPOSITORY_WORKFLOW_FILES {
            return Err(RepositoryWorkflowCatalogError::TooManyFiles);
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let mut paths = BTreeSet::new();
        for file in &files {
            file.validate_integrity()?;
            if !paths.insert(file.path.as_str()) {
                return Err(RepositoryWorkflowCatalogError::DuplicatePath(
                    file.path.as_str().to_string(),
                ));
            }
        }
        Ok(Self {
            repository_id,
            source_head_oid,
            source_change_version,
            state: RepositoryWorkflowCatalogState::Captured(files),
        })
    }

    pub fn rejected(
        repository_id: impl Into<String>,
        source_head_oid: impl Into<String>,
        source_change_version: u64,
        configuration_error: impl Into<String>,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        let repository_id = repository_id.into();
        let source_head_oid = source_head_oid.into().to_ascii_lowercase();
        validate_identity(&repository_id, &source_head_oid, source_change_version)?;
        let configuration_error = configuration_error.into();
        if configuration_error.trim().is_empty() {
            return Err(RepositoryWorkflowCatalogError::MissingConfigurationError);
        }
        if configuration_error.len() > MAX_REPOSITORY_WORKFLOW_CONFIGURATION_ERROR_BYTES {
            return Err(RepositoryWorkflowCatalogError::ConfigurationErrorTooLarge);
        }
        Ok(Self {
            repository_id,
            source_head_oid,
            source_change_version,
            state: RepositoryWorkflowCatalogState::Rejected(configuration_error),
        })
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn source_head_oid(&self) -> &str {
        &self.source_head_oid
    }

    pub fn source_change_version(&self) -> u64 {
        self.source_change_version
    }

    pub fn files(&self) -> Option<&[RepositoryWorkflowFile]> {
        match &self.state {
            RepositoryWorkflowCatalogState::Captured(files) => Some(files),
            RepositoryWorkflowCatalogState::Rejected(_) => None,
        }
    }

    pub fn configuration_error(&self) -> Option<&str> {
        match &self.state {
            RepositoryWorkflowCatalogState::Captured(_) => None,
            RepositoryWorkflowCatalogState::Rejected(error) => Some(error),
        }
    }

    pub fn validate_integrity(&self) -> Result<(), RepositoryWorkflowCatalogError> {
        validate_identity(
            &self.repository_id,
            &self.source_head_oid,
            self.source_change_version,
        )?;
        match &self.state {
            RepositoryWorkflowCatalogState::Captured(files) => {
                if files.len() > MAX_REPOSITORY_WORKFLOW_FILES {
                    return Err(RepositoryWorkflowCatalogError::TooManyFiles);
                }
                let mut paths = BTreeSet::new();
                for file in files {
                    file.validate_integrity()?;
                    if !paths.insert(file.path.as_str()) {
                        return Err(RepositoryWorkflowCatalogError::DuplicatePath(
                            file.path.as_str().to_string(),
                        ));
                    }
                }
                Ok(())
            }
            RepositoryWorkflowCatalogState::Rejected(error) => {
                if error.trim().is_empty() {
                    return Err(RepositoryWorkflowCatalogError::MissingConfigurationError);
                }
                if error.len() > MAX_REPOSITORY_WORKFLOW_CONFIGURATION_ERROR_BYTES {
                    return Err(RepositoryWorkflowCatalogError::ConfigurationErrorTooLarge);
                }
                Ok(())
            }
        }
    }

    pub fn verify_source(
        &self,
        repository_id: &str,
        source_head_oid: &str,
        source_change_version: u64,
    ) -> Result<(), RepositoryWorkflowCatalogError> {
        self.validate_integrity()?;
        if self.repository_id == repository_id
            && self.source_head_oid == source_head_oid.to_ascii_lowercase()
            && self.source_change_version == source_change_version
        {
            Ok(())
        } else {
            Err(RepositoryWorkflowCatalogError::SourceMismatch)
        }
    }

    pub fn rebind_source_change_version(
        mut self,
        repository_id: &str,
        source_head_oid: &str,
        source_change_version: u64,
    ) -> Result<Self, RepositoryWorkflowCatalogError> {
        validate_identity(repository_id, source_head_oid, source_change_version)?;
        if self.repository_id != repository_id
            || self.source_head_oid != source_head_oid.to_ascii_lowercase()
        {
            return Err(RepositoryWorkflowCatalogError::SourceMismatch);
        }
        self.source_change_version = source_change_version;
        Ok(self)
    }
}

fn validate_identity(
    repository_id: &str,
    source_head_oid: &str,
    source_change_version: u64,
) -> Result<(), RepositoryWorkflowCatalogError> {
    if repository_id.trim().is_empty() {
        return Err(RepositoryWorkflowCatalogError::MissingRepositoryId);
    }
    if !is_sha1_hex(source_head_oid) {
        return Err(RepositoryWorkflowCatalogError::InvalidSourceHead);
    }
    if source_change_version == 0 {
        return Err(RepositoryWorkflowCatalogError::InvalidSourceChangeVersion);
    }
    Ok(())
}

fn is_sha1_hex(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_blob_oid(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()));
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn source_blob_identity_matches(blob: &SourceBlob) -> bool {
    match &blob.content_ref {
        ContentRef::GitBlob { git_oid } => git_oid == &blob.git_oid,
        ContentRef::BlobSha256(sha256) => sha256 == &blob.sha256,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DEFAULT_GIT_FILE_MODE;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn workflow(path: &str, bytes: &[u8]) -> RepositoryWorkflowFile {
        RepositoryWorkflowFile::from_content(path, DEFAULT_GIT_FILE_MODE, bytes.to_vec()).unwrap()
    }

    #[test]
    fn captured_catalog_sorts_and_verifies_files() {
        let catalog = RepositoryWorkflowCatalog::captured(
            "repo-1",
            HEAD,
            7,
            vec![
                workflow("/.scope/runs/z-last.yml", b"last"),
                workflow("/.scope/runs/a-first.yaml", b"first"),
            ],
        )
        .unwrap();

        catalog.validate_integrity().unwrap();
        let paths = catalog
            .files()
            .unwrap()
            .iter()
            .map(|file| file.path().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            ["/.scope/runs/a-first.yaml", "/.scope/runs/z-last.yml"]
        );
        assert!(catalog.configuration_error().is_none());
        catalog.verify_source("repo-1", HEAD, 7).unwrap();
        assert!(matches!(
            catalog.verify_source("repo-1", HEAD, 8),
            Err(RepositoryWorkflowCatalogError::SourceMismatch)
        ));

        let rebound = catalog
            .rebind_source_change_version("repo-1", HEAD, 8)
            .unwrap();
        rebound.verify_source("repo-1", HEAD, 8).unwrap();
        assert!(matches!(
            rebound
                .clone()
                .rebind_source_change_version("other-repo", HEAD, 9),
            Err(RepositoryWorkflowCatalogError::SourceMismatch)
        ));
        assert!(matches!(
            rebound.rebind_source_change_version(
                "repo-1",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                9,
            ),
            Err(RepositoryWorkflowCatalogError::SourceMismatch)
        ));
    }

    #[test]
    fn file_rejects_corrupt_source_identity() {
        let error = RepositoryWorkflowFile::new(
            "/.scope/runs/checks.yml",
            HEAD,
            5,
            DEFAULT_GIT_FILE_MODE,
            b"wrong".to_vec(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RepositoryWorkflowCatalogError::GitObjectMismatch { .. }
        ));
    }

    #[test]
    fn file_builds_from_and_verifies_source_blob() {
        let bytes = b"workflow".to_vec();
        let oid = git_blob_oid(&bytes);
        let blob = SourceBlob {
            content_ref: ContentRef::git_blob(&oid),
            sha256: String::new(),
            git_oid: oid,
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: bytes.len() as u64,
        };

        let file =
            RepositoryWorkflowFile::from_source_blob("/.scope/runs/checks.yml", &blob, bytes)
                .unwrap();
        file.verify_source(&blob).unwrap();

        let mut wrong_source = blob;
        wrong_source.size_bytes += 1;
        assert!(matches!(
            file.verify_source(&wrong_source),
            Err(RepositoryWorkflowCatalogError::SourceBlobMismatch { .. })
        ));
    }

    #[test]
    fn catalog_rejects_duplicate_paths() {
        let file = workflow("/.scope/runs/checks.yml", b"checks");
        let error =
            RepositoryWorkflowCatalog::captured("repo-1", HEAD, 1, vec![file.clone(), file])
                .unwrap_err();

        assert!(matches!(
            error,
            RepositoryWorkflowCatalogError::DuplicatePath(path)
                if path == "/.scope/runs/checks.yml"
        ));
    }

    #[test]
    fn rejected_catalog_carries_no_files() {
        let catalog = RepositoryWorkflowCatalog::rejected(
            "repo-1",
            HEAD,
            2,
            "repository contains too many workflows",
        )
        .unwrap();

        assert!(catalog.files().is_none());
        assert_eq!(
            catalog.configuration_error(),
            Some("repository contains too many workflows")
        );
        catalog.validate_integrity().unwrap();
    }

    #[test]
    fn catalog_rejects_invalid_identity_and_empty_error() {
        assert!(matches!(
            RepositoryWorkflowCatalog::captured("repo-1", "not-an-oid", 1, Vec::new()),
            Err(RepositoryWorkflowCatalogError::InvalidSourceHead)
        ));
        assert!(matches!(
            RepositoryWorkflowCatalog::rejected("repo-1", HEAD, 1, "  "),
            Err(RepositoryWorkflowCatalogError::MissingConfigurationError)
        ));
        assert!(matches!(
            RepositoryWorkflowCatalog::rejected(
                "repo-1",
                HEAD,
                1,
                "x".repeat(MAX_REPOSITORY_WORKFLOW_CONFIGURATION_ERROR_BYTES + 1),
            ),
            Err(RepositoryWorkflowCatalogError::ConfigurationErrorTooLarge)
        ));
    }
}
