use super::{
    content::SourceBlob,
    policy::{ScopePath, Visibility},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityChangeSet {
    pub id: String,
    pub anchor_commit_id: Option<String>,
    pub source_update_id: Option<String>,
    pub author_id: String,
    pub changes: Vec<VisibilityChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityChange {
    pub path: ScopePath,
    pub old_visibility: Visibility,
    pub new_visibility: Visibility,
    pub current_content: Option<SourceBlob>,
}

impl VisibilityChangeSet {
    pub fn new(
        id: String,
        anchor_commit_id: Option<String>,
        source_update_id: Option<String>,
        author_id: String,
        changes: Vec<VisibilityChange>,
    ) -> Result<Self, &'static str> {
        if id.is_empty() || author_id.is_empty() {
            return Err("visibility change set id and author must not be empty");
        }
        if changes.is_empty() {
            return Err("visibility change set must contain at least one change");
        }
        if changes
            .iter()
            .any(|change| change.old_visibility == change.new_visibility)
        {
            return Err("visibility change set cannot contain no-op changes");
        }
        let unique_paths = changes
            .iter()
            .map(|change| &change.path)
            .collect::<BTreeSet<_>>();
        if unique_paths.len() != changes.len() {
            return Err("visibility change set cannot contain duplicate paths");
        }

        Ok(Self {
            id,
            anchor_commit_id,
            source_update_id,
            author_id,
            changes,
        })
    }
}

pub fn visibility_change_set_id(next_change_version: u64) -> String {
    format!("vchg_{next_change_version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(
        path: &str,
        old_visibility: Visibility,
        new_visibility: Visibility,
    ) -> VisibilityChange {
        VisibilityChange {
            path: ScopePath::parse(path).unwrap(),
            old_visibility,
            new_visibility,
            current_content: None,
        }
    }

    #[test]
    fn mixed_directions_are_one_valid_causal_set() {
        let set = VisibilityChangeSet::new(
            "vchg_2".into(),
            Some("rv1".into()),
            None,
            "owner".into(),
            vec![
                change("/public.md", Visibility::Private, Visibility::Public),
                change("/private.md", Visibility::Public, Visibility::Private),
            ],
        )
        .unwrap();

        assert_eq!(set.changes.len(), 2);
    }

    #[test]
    fn empty_duplicate_and_no_op_sets_are_rejected() {
        assert!(
            VisibilityChangeSet::new("vchg_2".into(), None, None, "owner".into(), Vec::new(),)
                .is_err()
        );
        assert!(
            VisibilityChangeSet::new(
                "vchg_2".into(),
                None,
                None,
                "owner".into(),
                vec![change("/same.md", Visibility::Public, Visibility::Public)],
            )
            .is_err()
        );
        assert!(
            VisibilityChangeSet::new(
                "vchg_2".into(),
                None,
                None,
                "owner".into(),
                vec![
                    change("/same.md", Visibility::Public, Visibility::Private),
                    change("/same.md", Visibility::Private, Visibility::Public),
                ],
            )
            .is_err()
        );
    }
}
