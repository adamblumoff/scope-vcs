use super::error::{ReviewedUpdateError, ReviewedUpdateResult};
use crate::{
    policy::{Policy, ScopePath, Visibility, VisibilityRule},
    repo_config::RepoConfig,
};

pub(super) fn policy_from_config_for_tree<'a>(
    config: &RepoConfig,
    paths: impl IntoIterator<Item = &'a ScopePath>,
) -> ReviewedUpdateResult<Policy> {
    let mut policy = Policy::new(config.visibility.default_visibility().into());
    for path in paths {
        let rule = match config.visibility_for_path(path) {
            Visibility::Public => VisibilityRule::public(path.clone()),
            Visibility::Private => VisibilityRule::private(path.clone()),
        };
        policy
            .add_rule(rule)
            .map_err(ReviewedUpdateError::InvalidPolicy)?;
    }
    Ok(policy)
}
