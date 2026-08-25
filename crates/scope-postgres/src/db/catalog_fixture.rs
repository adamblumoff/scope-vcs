//! Aggregate-shaped fixture data used only by Postgres tests and local development seeding.

use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    policy::Visibility,
    repo_actions::RepoStorageCleanup,
    repository::{CatalogError, Repository, repo_id},
    requests::{
        Request, RequestDiscussion, RequestDiscussionReadState, RequestDiscussionReply,
        RequestEvent, RequestRevision,
    },
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct CatalogFixture {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, Repository>,
    pub requests: BTreeMap<String, Request>,
    pub request_revisions: BTreeMap<String, RequestRevision>,
    pub request_discussions: BTreeMap<String, RequestDiscussion>,
    pub request_discussion_replies: BTreeMap<String, RequestDiscussionReply>,
    pub request_discussion_read_states: BTreeMap<String, RequestDiscussionReadState>,
    pub request_events: BTreeMap<String, RequestEvent>,
    pub pending_repo_storage_deletions: Vec<RepoStorageCleanup>,
    pub pending_source_blob_deletions: Vec<SourceBlob>,
}

impl CatalogFixture {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&Repository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn repositories_for_user(&self, user_id: &str) -> Vec<&Repository> {
        self.repositories
            .values()
            .filter(|repo| {
                repo.record.owner_user_id == user_id
                    || repo.members.iter().any(|member| member.user_id == user_id)
            })
            .collect()
    }

    pub fn create_repository(
        &mut self,
        owner: &UserAccount,
        name: &str,
        default_visibility: Visibility,
    ) -> Result<&Repository, CatalogError> {
        let repository = Repository::new(owner, name, default_visibility)?;
        let id = repository.record.id.clone();
        self.repositories.insert(id.clone(), repository);
        Ok(self.repositories.get(&id).expect("repository was inserted"))
    }
}
