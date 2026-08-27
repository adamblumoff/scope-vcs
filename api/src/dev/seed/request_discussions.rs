use crate::error::ApiError;
use scope_domain::{
    account::UserAccount,
    repository::Repository,
    repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput, RequestDiscussionAnchor,
        RequestDiscussionStatus,
    },
};
use scope_postgres::db::MetadataStore;

pub(super) const CONTRIBUTOR_ID: &str = "scope_usr_dev_contributor";
pub(super) const MAINTAINER_ID: &str = "scope_usr_dev_maintainer";
pub(super) const REQUEST_ID: &str = "req_demo_ready";
pub(super) const READY_REQUEST_DESCRIPTION: &str = concat!(
    "This request adds bounded retry timing to the remote client.\n\n",
    "The implementation caps exponential backoff at two seconds and keeps the ",
    "helper small enough to reuse from fetch and push paths. Please focus review ",
    "on the cap, jitter behavior, and whether the exported API communicates units."
);
pub(super) const RETRY_CAP_ID: &str = "discussion_demo_retry_cap";
pub(super) const RETRY_CAP_MAINTAINER_REPLY_ID: &str = "discussion_reply_demo_retry_cap_maintainer";
const RETRY_CAP_CONTRIBUTOR_REPLY_ID: &str = "discussion_reply_demo_retry_cap_quote";
pub(super) const RESOLVED_DOCS_ID: &str = "discussion_demo_resolved_docs";
const JITTER_ID: &str = "discussion_demo_jitter";
const JITTER_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_jitter";
const TEST_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_tests";
const FINAL_REVISION_DISCUSSION_ID: &str = "discussion_demo_revision_final";

struct SeedRevisionDiscussion {
    discussion_id: &'static str,
    revision_id: &'static str,
    actor_user_id: &'static str,
    client_discussion_id: &'static str,
    body: &'static str,
}

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

pub(super) fn add_maintainer(repo: &mut Repository) {
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
    create_retry_cap_conversation(metadata).await?;
    create_jitter_conversation(metadata).await?;
    create_resolved_docs_conversation(metadata).await?;
    create_revision_conversations(metadata).await?;

    let resolved = metadata
        .requests()
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

async fn create_revision_conversations(metadata: &MetadataStore) -> Result<(), ApiError> {
    let discussions = [
        SeedRevisionDiscussion {
            discussion_id: JITTER_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_2",
            actor_user_id: MAINTAINER_ID,
            client_discussion_id: "seed_revision_jitter",
            body: concat!(
                "The bounded jitter looks right. A deterministic random source in the tests ",
                "would make the boundary behavior easy to review."
            ),
        },
        SeedRevisionDiscussion {
            discussion_id: TEST_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_3",
            actor_user_id: CONTRIBUTOR_ID,
            client_discussion_id: "seed_revision_tests",
            body: "The tests now pin both the zero-jitter and maximum-jitter edges.",
        },
        SeedRevisionDiscussion {
            discussion_id: FINAL_REVISION_DISCUSSION_ID,
            revision_id: "event_req_demo_ready_revision_4",
            actor_user_id: MAINTAINER_ID,
            client_discussion_id: "seed_revision_final",
            body: concat!(
                "This closes the loop nicely: exported policy, implementation, tests, and ",
                "contributor documentation all agree."
            ),
        },
    ];
    for (index, discussion) in discussions.into_iter().enumerate() {
        metadata
            .requests()
            .create_request_discussion(CreateRequestDiscussionInput {
                request_id: REQUEST_ID.to_string(),
                id: discussion.discussion_id.to_string(),
                actor_user_id: discussion.actor_user_id.to_string(),
                actor_can_participate: false,
                client_discussion_id: discussion.client_discussion_id.to_string(),
                body_markdown: discussion.body.to_string(),
                anchor: Some(RequestDiscussionAnchor {
                    revision_id: discussion.revision_id.to_string(),
                    commit_oid: None,
                    path: None,
                }),
                now_unix: 1_800_000_150 + index as u64,
            })
            .await?;
    }
    Ok(())
}

async fn create_retry_cap_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .requests()
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
            anchor: None,
            now_unix: 1_800_000_120,
        })
        .await?;
    metadata
        .requests()
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
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: RETRY_CAP_CONTRIBUTOR_REPLY_ID.to_string(),
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
    metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: REQUEST_ID.to_string(),
            discussion_id: RETRY_CAP_ID.to_string(),
            id: "discussion_reply_demo_retry_cap_nested".to_string(),
            actor_user_id: MAINTAINER_ID.to_string(),
            actor_can_participate: false,
            client_reply_id: "seed_retry_cap_nested".to_string(),
            body_markdown: concat!(
                "Exactly. Keeping that decision nested here makes the implementation history ",
                "easy to follow without expanding the whole conversation."
            )
            .to_string(),
            reply_to_reply_id: Some(RETRY_CAP_CONTRIBUTOR_REPLY_ID.to_string()),
            now_unix: 1_800_000_123,
        })
        .await?;
    Ok(())
}

async fn create_jitter_conversation(metadata: &MetadataStore) -> Result<(), ApiError> {
    metadata
        .requests()
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
            anchor: None,
            now_unix: 1_800_000_130,
        })
        .await?;
    metadata
        .requests()
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
        .requests()
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
            anchor: None,
            now_unix: 1_800_000_140,
        })
        .await?;
    metadata
        .requests()
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
        .requests()
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
    use crate::demo_seed::DevSeedUser;
    use scope_domain::requests::RequestEventKind;
    use scope_object_store::{EncryptedObjectStore, MemoryObjectStore};
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
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        metadata.admin().seed_catalog_for_tests(catalog).unwrap();

        seed_request_discussion_gallery(&metadata).await.unwrap();

        let request = metadata
            .requests()
            .request_by_id(REQUEST_ID)
            .await
            .unwrap()
            .unwrap();
        assert!(
            request
                .description_markdown
                .contains("bounded retry timing")
        );

        let discussions = metadata
            .requests()
            .request_discussions_page(scope_postgres::db::RequestDiscussionsPageQuery {
                request_id: REQUEST_ID,
                viewer_user_id: Some(super::super::DEV_SEED_USER_ID),
                snapshot_version: request.activity_version,
                cursor: None,
                discussion_id: None,
                anchor_revision_id: None,
                anchor_commit_oid: None,
                include_revision_anchor: false,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(discussions.discussions.len(), 6);
        assert_eq!(
            discussions
                .discussions
                .iter()
                .map(|model| model.discussion.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                FINAL_REVISION_DISCUSSION_ID,
                TEST_REVISION_DISCUSSION_ID,
                JITTER_REVISION_DISCUSSION_ID,
                RESOLVED_DOCS_ID,
                JITTER_ID,
                RETRY_CAP_ID,
            ]
        );
        assert_eq!(
            discussions
                .discussions
                .iter()
                .filter(|model| model.discussion.status == RequestDiscussionStatus::Open)
                .count(),
            5
        );
        assert_eq!(
            discussions
                .discussions
                .iter()
                .filter(|model| model.discussion.anchor.is_some())
                .count(),
            3
        );
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
        let retry_cap = discussions
            .discussions
            .iter()
            .find(|model| model.discussion.id == RETRY_CAP_ID)
            .unwrap();
        assert_eq!(retry_cap.reply_count, 3);
        assert_eq!(retry_cap.latest_replies.len(), 3);
        assert!(retry_cap.latest_replies[0].reply_to.is_none());
        assert_eq!(
            retry_cap.latest_replies[1].reply_to.as_ref().unwrap().id,
            RETRY_CAP_MAINTAINER_REPLY_ID
        );

        let (replies, users) = metadata
            .requests()
            .request_discussion_replies(RETRY_CAP_ID, None, 10)
            .await
            .unwrap();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0].reply.id, RETRY_CAP_MAINTAINER_REPLY_ID);
        assert_eq!(
            replies[1].reply.reply_to_reply_id.as_deref(),
            Some(RETRY_CAP_MAINTAINER_REPLY_ID)
        );
        assert_eq!(
            replies[2].reply_to.as_ref().unwrap().id,
            RETRY_CAP_CONTRIBUTOR_REPLY_ID
        );
        assert_eq!(
            users.get(CONTRIBUTOR_ID).unwrap().handle,
            "river-contributor"
        );
        assert_eq!(users.get(MAINTAINER_ID).unwrap().handle, "maya-maintainer");

        let activity = metadata
            .requests()
            .request_events_by_request_id(REQUEST_ID)
            .await
            .unwrap();
        assert!(
            activity
                .iter()
                .any(|event| event.kind == RequestEventKind::IdentityEdited)
        );
        assert!(
            activity
                .iter()
                .any(|event| event.kind == RequestEventKind::DiscussionResolved)
        );
    }
}
