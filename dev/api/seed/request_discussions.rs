use crate::{
    db::MetadataStore,
    domain::{
        requests::{
            CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput,
            RequestDiscussionStatus, UpdateRequestDescriptionInput,
        },
        store::{RepositoryMember, RepositoryMemberPermissions, StoredRepository, UserAccount},
    },
    error::ApiError,
};

pub(super) const CONTRIBUTOR_ID: &str = "scope_usr_dev_contributor";
pub(super) const MAINTAINER_ID: &str = "scope_usr_dev_maintainer";
pub(super) const REQUEST_ID: &str = "req_demo_submitted";
pub(super) const RETRY_CAP_ID: &str = "discussion_demo_retry_cap";
pub(super) const RETRY_CAP_MAINTAINER_REPLY_ID: &str = "discussion_reply_demo_retry_cap_maintainer";
pub(super) const RESOLVED_DOCS_ID: &str = "discussion_demo_resolved_docs";
const JITTER_ID: &str = "discussion_demo_jitter";

pub(super) fn collaborators() -> [UserAccount; 2] {
    [
        user(
            CONTRIBUTOR_ID,
            "river-contributor",
            "river.contributor@example.test",
        ),
        user(
            MAINTAINER_ID,
            "maya-maintainer",
            "maya.maintainer@example.test",
        ),
    ]
}

pub(super) fn add_maintainer(repo: &mut StoredRepository) {
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: MAINTAINER_ID.to_string(),
        permissions: RepositoryMemberPermissions {
            can_push: true,
            can_change_file_visibility: true,
            can_apply_changes: true,
        },
        created_at_unix: 1_800_000_000,
        updated_at_unix: 1_800_000_000,
    });
}

pub(crate) async fn seed_request_discussion_gallery(
    metadata: &MetadataStore,
) -> Result<(), ApiError> {
    metadata
        .update_request_description(UpdateRequestDescriptionInput {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            actor_can_edit_description: false,
            event_id: "event_demo_description_edited".to_string(),
            description_markdown: concat!(
                "This request adds bounded retry timing to the remote client.\n\n",
                "The implementation caps exponential backoff at two seconds and keeps the ",
                "helper small enough to reuse from fetch and push paths. Please focus review ",
                "on the cap, jitter behavior, and whether the exported API communicates units."
            )
            .to_string(),
            now_unix: 1_800_000_110,
        })
        .await?;

    create_retry_cap_conversation(metadata).await?;
    create_jitter_conversation(metadata).await?;
    create_resolved_docs_conversation(metadata).await?;

    let resolved = metadata
        .request_discussion(REQUEST_ID, RESOLVED_DOCS_ID, Some(super::DEV_SEED_USER_ID))
        .await?
        .ok_or_else(|| ApiError::internal_message("seeded resolved discussion is missing"))?;
    if resolved.0.discussion.status != RequestDiscussionStatus::Resolved {
        return Err(ApiError::internal_message(
            "seeded resolved discussion did not resolve",
        ));
    }
    Ok(())
}

async fn create_retry_cap_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: REQUEST_ID.to_string(),
            id: RETRY_CAP_ID.to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            actor_can_participate: false,
            client_discussion_id: "seed_retry_cap".to_string(),
            body_markdown: concat!(
                "Should the retry cap remain **2 seconds**, or should callers be able to ",
                "override it? I like the predictable default, but an explicit constant might ",
                "make the policy easier to discover."
            )
            .to_string(),
            now_unix: 1_800_000_120,
        })
        .await?;
    metadata
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: RETRY_CAP_MAINTAINER_REPLY_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            actor_can_participate: false,
            client_reply_id: "seed_retry_cap_maintainer".to_string(),
            body_markdown: concat!(
                "Two seconds is intentional for interactive commands. Let's extract ",
                "`MAX_RETRY_DELAY_MS` and leave per-command overrides out of this request."
            )
            .to_string(),
            reply_to_reply_id: None,
            now_unix: 1_800_000_121,
        })
        .await?;
    metadata
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: "discussion_reply_demo_retry_cap_quote".to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            actor_can_participate: false,
            client_reply_id: "seed_retry_cap_quote".to_string(),
            body_markdown: concat!(
                "Agreed. Quoting the maintainer response here so the decision remains ",
                "attached to the suggestion: keep the fixed cap and name the constant."
            )
            .to_string(),
            reply_to_reply_id: Some(RETRY_CAP_MAINTAINER_REPLY_ID.to_string()),
            now_unix: 1_800_000_122,
        })
        .await?;
    Ok(())
}

async fn create_jitter_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: REQUEST_ID.to_string(),
            id: JITTER_ID.to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            actor_can_participate: false,
            client_discussion_id: "seed_jitter".to_string(),
            body_markdown: concat!(
                "Can we add a small amount of jitter before this lands? Simultaneous clients ",
                "currently retry on exactly the same boundaries."
            )
            .to_string(),
            now_unix: 1_800_000_130,
        })
        .await?;
    metadata
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: JITTER_ID.to_string(),
            id: "discussion_reply_demo_jitter".to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            actor_can_participate: false,
            client_reply_id: "seed_jitter_reply".to_string(),
            body_markdown: concat!(
                "Yes. I'll use bounded positive jitter and add a deterministic unit test ",
                "around the range rather than snapshotting random values."
            )
            .to_string(),
            reply_to_reply_id: None,
            now_unix: 1_800_000_131,
        })
        .await?;
    Ok(())
}

async fn create_resolved_docs_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: REQUEST_ID.to_string(),
            id: RESOLVED_DOCS_ID.to_string(),
            actor_user_id: CONTRIBUTOR_ID.to_string(),
            actor_can_participate: false,
            client_discussion_id: "seed_resolved_docs".to_string(),
            body_markdown: concat!(
                "The helper accepts milliseconds, but the name `retryDelay` does not state ",
                "the unit. Could the doc comment make that explicit?"
            )
            .to_string(),
            now_unix: 1_800_000_140,
        })
        .await?;
    metadata
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RESOLVED_DOCS_ID.to_string(),
            id: "discussion_reply_demo_resolved_docs".to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            actor_can_participate: false,
            client_reply_id: "seed_resolved_docs_reply".to_string(),
            body_markdown: "The new doc comment now says the returned delay is in milliseconds."
                .to_string(),
            reply_to_reply_id: None,
            now_unix: 1_800_000_141,
        })
        .await?;
    metadata
        .resolve_request_discussion(
            REQUEST_ID.to_string(),
            RESOLVED_DOCS_ID.to_string(),
            MAINTAINER_ID.to_string(),
            "event_demo_discussion_resolved".to_string(),
            1_800_000_142,
        )
        .await?;
    Ok(())
}

fn user(id: &str, handle: &str, email: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dev::env::DevSeedUser,
        domain::requests::RequestEventKind,
        object_store::{EncryptedObjectStore, MemoryObjectStore},
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn gallery_covers_open_resolved_and_quoted_conversations() {
        let object_store = EncryptedObjectStore::new(Arc::new(MemoryObjectStore::new()), [9; 32]);
        let catalog = super::super::catalog(
            &object_store,
            DevSeedUser {
                email: "dev@example.com".to_string(),
                handle: "dev".to_string(),
            },
        )
        .unwrap();
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        metadata
            .replace_catalog_for_local_dev(catalog)
            .await
            .unwrap();

        seed_request_discussion_gallery(&metadata).await.unwrap();

        let request = metadata.request_by_id(REQUEST_ID).await.unwrap().unwrap();
        assert!(
            request
                .description_markdown
                .contains("bounded retry timing")
        );

        let discussions = metadata
            .request_discussions_page(scope_core::db::RequestDiscussionsPageQuery {
                request_id: REQUEST_ID,
                viewer_user_id: Some(super::super::DEV_SEED_USER_ID),
                status: None,
                recent: true,
                snapshot_version: request.activity_version,
                cursor: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(discussions.discussions.len(), 4);
        assert_eq!(
            discussions
                .discussions
                .iter()
                .filter(|model| model.discussion.status == RequestDiscussionStatus::Open)
                .count(),
            2
        );
        let change_block = discussions
            .discussions
            .iter()
            .find(|model| model.discussion.status == RequestDiscussionStatus::Dormant)
            .unwrap();
        assert!(change_block.change_block.is_some());
        assert!(change_block.discussion.body_markdown.is_none());
        assert_eq!(
            discussions
                .discussions
                .iter()
                .find(|model| model.discussion.id == RESOLVED_DOCS_ID)
                .unwrap()
                .discussion
                .status,
            RequestDiscussionStatus::Resolved
        );

        let (replies, users) = metadata
            .request_discussion_replies(RETRY_CAP_ID, None, 10)
            .await
            .unwrap();
        assert_eq!(replies.len(), 2);
        assert_eq!(
            replies
                .iter()
                .find(|reply| reply.reply_to_reply_id.is_some())
                .unwrap()
                .reply_to_reply_id
                .as_deref(),
            Some(RETRY_CAP_MAINTAINER_REPLY_ID)
        );
        assert_eq!(
            users.get(CONTRIBUTOR_ID).unwrap().handle,
            "river-contributor"
        );
        assert_eq!(users.get(MAINTAINER_ID).unwrap().handle, "maya-maintainer");

        let activity = metadata
            .request_events_by_request_id(REQUEST_ID)
            .await
            .unwrap();
        assert!(
            activity
                .iter()
                .any(|event| event.kind == RequestEventKind::DescriptionEdited)
        );
        assert!(
            activity
                .iter()
                .any(|event| event.kind == RequestEventKind::DiscussionResolved)
        );
    }
}
