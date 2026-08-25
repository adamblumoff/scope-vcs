use super::validation::validate_sha256_hash;
use crate::{
    content::SourceBlob,
    content_ref::ContentRef,
    error::DomainError,
    projection::ProjectionViewKey,
    repository::git::{GitHead, GitPackSpan, validate_git_pack_layout},
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
    AcceptedGitHead {
        repository_id: String,
        head: GitHead,
        pack_spans: Vec<GitPackSpan>,
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

    pub fn accepted_git_head(
        repository_id: impl Into<String>,
        head: GitHead,
        pack_spans: Vec<GitPackSpan>,
        audience: ProjectionViewKey,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "accepted run source repository id is required",
            ));
        }
        if head.push_sequence == 0 || head.change_version == 0 {
            return Err(DomainError::invalid_input(
                "accepted run source sequence and change version must be positive",
            ));
        }
        validate_git_oid("accepted run source head", &head.head_oid)?;
        validate_source_blob(&head.manifest, "run source manifest")?;
        if !matches!(head.manifest.content_ref, ContentRef::GitManifestSha256(_)) {
            return Err(DomainError::invalid_input(
                "accepted run source must use a Git manifest",
            ));
        }
        if head.manifest.git_oid != head.head_oid {
            return Err(DomainError::invalid_input(
                "accepted run source manifest head does not match the accepted head",
            ));
        }
        if pack_spans.is_empty() {
            return Err(DomainError::invalid_input(
                "accepted run source must pin at least one Git pack span",
            ));
        }
        validate_git_pack_layout(&pack_spans)
            .map_err(|error| DomainError::invalid_input(error.to_string()))?;
        for span in &pack_spans {
            validate_source_blob(&span.object, "run source Git pack")?;
            if !matches!(span.object.content_ref, ContentRef::GitSegmentSha256(_)) {
                return Err(DomainError::invalid_input(
                    "accepted run source packs must be Git segments",
                ));
            }
        }
        let final_span = pack_spans.last().expect("empty pack spans were rejected");
        if final_span.last_sequence != head.push_sequence || final_span.head_oid != head.head_oid {
            return Err(DomainError::invalid_input(
                "accepted run source packs do not reach the accepted head",
            ));
        }
        Ok(Self::AcceptedGitHead {
            repository_id,
            head,
            pack_spans,
            audience,
        })
    }

    pub fn ephemeral_bundle(&self) -> Option<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => Some(object),
            Self::AcceptedGitHead { .. } => None,
        }
    }

    pub fn retained_objects(&self) -> Vec<&SourceBlob> {
        match self {
            Self::EphemeralGitBundle { object } => vec![object],
            Self::AcceptedGitHead {
                head, pack_spans, ..
            } => std::iter::once(&head.manifest)
                .chain(pack_spans.iter().map(|span| &span.object))
                .collect(),
        }
    }

    pub fn source_identity(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } => &object.sha256,
            Self::AcceptedGitHead { head, .. } => &head.manifest.sha256,
        }
    }

    pub fn git_oid(&self) -> &str {
        match self {
            Self::EphemeralGitBundle { object } => &object.git_oid,
            Self::AcceptedGitHead { head, .. } => &head.head_oid,
        }
    }

    pub fn logical_git_head(&self) -> Option<(&str, &GitHead, &[GitPackSpan])> {
        match self {
            Self::AcceptedGitHead {
                repository_id,
                head,
                pack_spans,
                ..
            } => Some((repository_id, head, pack_spans)),
            Self::EphemeralGitBundle { .. } => None,
        }
    }

    pub fn is_private_only(&self) -> bool {
        matches!(
            self,
            Self::EphemeralGitBundle { .. }
                | Self::AcceptedGitHead {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::DEFAULT_GIT_FILE_MODE;

    #[test]
    fn accepted_git_head_pins_the_exact_pack_layout() {
        let head_oid = "a".repeat(40);
        let manifest = source_blob(
            ContentRef::git_manifest_sha256("b".repeat(64)),
            'b',
            &head_oid,
        );
        let pack = GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: head_oid.clone(),
            object: source_blob(
                ContentRef::git_segment_sha256("c".repeat(64)),
                'c',
                &head_oid,
            ),
        };
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead {
                head_oid: head_oid.clone(),
                push_sequence: 1,
                change_version: 7,
                manifest: manifest.clone(),
            },
            vec![pack.clone()],
            ProjectionViewKey::Private,
        )
        .unwrap();

        assert_eq!(source.git_oid(), head_oid);
        assert_eq!(source.source_identity(), manifest.sha256);
        assert_eq!(source.retained_objects(), vec![&manifest, &pack.object]);
    }

    #[test]
    fn accepted_git_head_rejects_a_frontier_that_does_not_match_its_head() {
        let head_oid = "a".repeat(40);
        let source = RunSource::accepted_git_head(
            "owner/repo",
            GitHead {
                head_oid: head_oid.clone(),
                push_sequence: 2,
                change_version: 7,
                manifest: source_blob(
                    ContentRef::git_manifest_sha256("b".repeat(64)),
                    'b',
                    &head_oid,
                ),
            },
            vec![GitPackSpan {
                first_sequence: 1,
                last_sequence: 1,
                geometric_tier: 0,
                base_oid: None,
                head_oid: head_oid.clone(),
                object: source_blob(
                    ContentRef::git_segment_sha256("c".repeat(64)),
                    'c',
                    &head_oid,
                ),
            }],
            ProjectionViewKey::Private,
        );

        assert!(source.unwrap_err().message.contains("do not reach"));
    }

    fn source_blob(content_ref: ContentRef, character: char, git_oid: &str) -> SourceBlob {
        SourceBlob {
            content_ref,
            sha256: character.to_string().repeat(64),
            git_oid: git_oid.to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        }
    }
}
