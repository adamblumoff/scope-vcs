//! Aggregate-shaped fixture data used only by Postgres tests and local development seeding.

use scope_domain::{
    policy::Visibility,
    requests::{
        Request, RequestChangeBlock, RequestDiscussion, RequestDiscussionReadState,
        RequestDiscussionReply, RequestEvent,
    },
    store::{CatalogError, RepoStorageCleanup, SourceBlob, StoredRepository, UserAccount, repo_id},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct CatalogFixture {
    pub users: BTreeMap<String, UserAccount>,
    pub repositories: BTreeMap<String, StoredRepository>,
    pub requests: BTreeMap<String, Request>,
    pub request_change_blocks: BTreeMap<String, RequestChangeBlock>,
    pub request_discussions: BTreeMap<String, RequestDiscussion>,
    pub request_discussion_replies: BTreeMap<String, RequestDiscussionReply>,
    pub request_discussion_read_states: BTreeMap<String, RequestDiscussionReadState>,
    pub request_events: BTreeMap<String, RequestEvent>,
    pub pending_repo_storage_deletions: Vec<RepoStorageCleanup>,
    pub pending_source_blob_deletions: Vec<SourceBlob>,
}

impl CatalogFixture {
    pub fn repository(&self, owner: &str, name: &str) -> Option<&StoredRepository> {
        self.repositories.get(&repo_id(owner, name))
    }

    pub fn repositories_for_user(&self, user_id: &str) -> Vec<&StoredRepository> {
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
    ) -> Result<&StoredRepository, CatalogError> {
        let repository = StoredRepository::new(owner, name, default_visibility)?;
        let id = repository.record.id.clone();
        self.repositories.insert(id.clone(), repository);
        Ok(self.repositories.get(&id).expect("repository was inserted"))
    }
}
