use super::error::{ReviewedUpdateError, ReviewedUpdateResult};
use crate::{
    policy::{Policy, ScopePath, VisibilityRule},
    repo_config::RepoConfig,
};

pub(super) fn policy_from_config_for_tree<'a>(
    config: &RepoConfig,
    paths: impl IntoIterator<Item = &'a ScopePath>,
) -> ReviewedUpdateResult<Policy> {
    let mut policy = Policy::new(config.visibility.default_visibility().into());
    policy
        .add_rules(paths.into_iter().map(|path| VisibilityRule {
            path: path.clone(),
            visibility: config.visibility_for_path(path),
        }))
        .map_err(ReviewedUpdateError::InvalidPolicy)?;
    Ok(policy)
}
