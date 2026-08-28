use crate::content::SourceBlob;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHead {
    pub head_oid: String,
    pub push_sequence: u64,
    pub change_version: u64,
    pub manifest: SourceBlob,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPackSpan {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub geometric_tier: u32,
    pub base_oid: Option<String>,
    pub head_oid: String,
    pub segment: GitSegmentRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSegmentRef {
    pub segment_id: String,
    pub sha256: String,
    pub plaintext_bytes: u64,
    pub encoding_version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitSegmentUploadState {
    Uploading,
    Ready,
    Published,
    Deleting,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSegmentUpload {
    pub segment_id: String,
    pub repository_id: String,
    pub object_key: String,
    pub state: GitSegmentUploadState,
    pub sha256: Option<String>,
    pub plaintext_bytes: Option<u64>,
    pub encrypted_bytes: Option<u64>,
    pub encoding_version: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl GitPackSpan {
    pub fn sequence_count(&self) -> Result<u64, GitPackLayoutError> {
        self.last_sequence
            .checked_sub(self.first_sequence)
            .and_then(|distance| distance.checked_add(1))
            .ok_or(GitPackLayoutError::InvalidRange {
                first_sequence: self.first_sequence,
                last_sequence: self.last_sequence,
            })
    }

    pub fn expected_geometric_tier(&self) -> Result<u32, GitPackLayoutError> {
        let sequence_count = self.sequence_count()?;
        if !sequence_count.is_power_of_two() {
            return Err(GitPackLayoutError::NonGeometricCoverage {
                first_sequence: self.first_sequence,
                last_sequence: self.last_sequence,
                sequence_count,
            });
        }
        Ok(sequence_count.ilog2())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GitPackLayoutError {
    #[error("Git pack span range {first_sequence}..{last_sequence} is invalid")]
    InvalidRange {
        first_sequence: u64,
        last_sequence: u64,
    },
    #[error(
        "Git pack span {first_sequence}..{last_sequence} has geometric tier {actual}, expected {expected}"
    )]
    InvalidGeometricTier {
        first_sequence: u64,
        last_sequence: u64,
        actual: u32,
        expected: u32,
    },
    #[error(
        "Git pack span {first_sequence}..{last_sequence} covers {sequence_count} pushes instead of a power of two"
    )]
    NonGeometricCoverage {
        first_sequence: u64,
        last_sequence: u64,
        sequence_count: u64,
    },
    #[error("Git pack layout must start at sequence 1, found {first_sequence}")]
    InvalidStart { first_sequence: u64 },
    #[error("Git pack layout starts with a non-empty base object {base_oid}")]
    InvalidStartBase { base_oid: String },
    #[error(
        "Git pack layout has a gap or overlap between sequences {previous_last_sequence} and {next_first_sequence}"
    )]
    NonContiguous {
        previous_last_sequence: u64,
        next_first_sequence: u64,
    },
    #[error(
        "Git pack span history is disconnected: previous head {previous_head_oid}, next base {next_base_oid:?}"
    )]
    DisconnectedHistory {
        previous_head_oid: String,
        next_base_oid: Option<String>,
    },
    #[error(
        "Git pack span tiers must be non-increasing from oldest to newest, found {previous_tier} before {next_tier}"
    )]
    IncreasingTier { previous_tier: u32, next_tier: u32 },
}

pub fn validate_git_pack_layout(spans: &[GitPackSpan]) -> Result<(), GitPackLayoutError> {
    let Some(first) = spans.first() else {
        return Ok(());
    };
    if first.first_sequence != 1 {
        return Err(GitPackLayoutError::InvalidStart {
            first_sequence: first.first_sequence,
        });
    }
    if let Some(base_oid) = &first.base_oid {
        return Err(GitPackLayoutError::InvalidStartBase {
            base_oid: base_oid.clone(),
        });
    }

    validate_git_pack_span_run(spans)
}

pub fn validate_git_pack_span_run(spans: &[GitPackSpan]) -> Result<(), GitPackLayoutError> {
    for (index, span) in spans.iter().enumerate() {
        let expected = span.expected_geometric_tier()?;
        if span.geometric_tier != expected {
            return Err(GitPackLayoutError::InvalidGeometricTier {
                first_sequence: span.first_sequence,
                last_sequence: span.last_sequence,
                actual: span.geometric_tier,
                expected,
            });
        }
        if let Some(previous) = index.checked_sub(1).and_then(|index| spans.get(index)) {
            let expected_first =
                previous
                    .last_sequence
                    .checked_add(1)
                    .ok_or(GitPackLayoutError::InvalidRange {
                        first_sequence: previous.first_sequence,
                        last_sequence: previous.last_sequence,
                    })?;
            if span.first_sequence != expected_first {
                return Err(GitPackLayoutError::NonContiguous {
                    previous_last_sequence: previous.last_sequence,
                    next_first_sequence: span.first_sequence,
                });
            }
            if span.base_oid.as_deref() != Some(previous.head_oid.as_str()) {
                return Err(GitPackLayoutError::DisconnectedHistory {
                    previous_head_oid: previous.head_oid.clone(),
                    next_base_oid: span.base_oid.clone(),
                });
            }
            if span.geometric_tier > previous.geometric_tier {
                return Err(GitPackLayoutError::IncreasingTier {
                    previous_tier: previous.geometric_tier,
                    next_tier: span.geometric_tier,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack_span(first_sequence: u64, last_sequence: u64, geometric_tier: u32) -> GitPackSpan {
        GitPackSpan {
            first_sequence,
            last_sequence,
            geometric_tier,
            base_oid: (first_sequence > 1).then(|| format!("head-{}", first_sequence - 1)),
            head_oid: format!("head-{last_sequence}"),
            segment: GitSegmentRef {
                segment_id: format!("segment-{first_sequence}"),
                sha256: format!("pack-{first_sequence}"),
                plaintext_bytes: 1,
                encoding_version: 2,
            },
        }
    }

    #[test]
    fn git_pack_layout_accepts_contiguous_geometric_spans() {
        let spans = [
            pack_span(1, 8, 3),
            pack_span(9, 12, 2),
            pack_span(13, 13, 0),
        ];

        validate_git_pack_layout(&spans).unwrap();
    }

    #[test]
    fn git_pack_layout_rejects_gaps_and_overlaps() {
        let gap = [pack_span(1, 4, 2), pack_span(6, 6, 0)];
        assert_eq!(
            validate_git_pack_layout(&gap).unwrap_err(),
            GitPackLayoutError::NonContiguous {
                previous_last_sequence: 4,
                next_first_sequence: 6,
            }
        );

        let overlap = [pack_span(1, 4, 2), pack_span(4, 4, 0)];
        assert_eq!(
            validate_git_pack_layout(&overlap).unwrap_err(),
            GitPackLayoutError::NonContiguous {
                previous_last_sequence: 4,
                next_first_sequence: 4,
            }
        );
    }

    #[test]
    fn git_pack_layout_rejects_a_tier_that_does_not_match_coverage() {
        let spans = [pack_span(1, 8, 2)];

        assert_eq!(
            validate_git_pack_layout(&spans).unwrap_err(),
            GitPackLayoutError::InvalidGeometricTier {
                first_sequence: 1,
                last_sequence: 8,
                actual: 2,
                expected: 3,
            }
        );
    }

    #[test]
    fn git_pack_layout_requires_power_of_two_coverage_in_descending_tier_order() {
        let uneven = [pack_span(1, 6, 2)];
        assert_eq!(
            validate_git_pack_layout(&uneven).unwrap_err(),
            GitPackLayoutError::NonGeometricCoverage {
                first_sequence: 1,
                last_sequence: 6,
                sequence_count: 6,
            }
        );

        let increasing = [pack_span(1, 1, 0), pack_span(2, 3, 1)];
        assert_eq!(
            validate_git_pack_layout(&increasing).unwrap_err(),
            GitPackLayoutError::IncreasingTier {
                previous_tier: 0,
                next_tier: 1,
            }
        );
    }

    #[test]
    fn git_pack_layout_requires_a_connected_history_boundary() {
        let mut first = pack_span(1, 2, 1);
        first.base_oid = Some("unexpected".to_string());
        assert!(matches!(
            validate_git_pack_layout(&[first]).unwrap_err(),
            GitPackLayoutError::InvalidStartBase { .. }
        ));

        let mut disconnected = [pack_span(1, 2, 1), pack_span(3, 3, 0)];
        disconnected[1].base_oid = Some("different".to_string());
        assert!(matches!(
            validate_git_pack_layout(&disconnected).unwrap_err(),
            GitPackLayoutError::DisconnectedHistory { .. }
        ));
    }
}
