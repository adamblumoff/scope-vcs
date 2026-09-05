use super::request_discussion_commands::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand, DiscussionTransition,
    ReopenAndReplyToRequestDiscussionCommand, TransitionRequestDiscussionCommand,
};
use super::{
    RequestStore,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_discussion_rows::{
        DiscussionPageFilter, RequestDiscussionReplyReadModel,
        RequestDiscussionReplyReferenceReadModel, changed_discussions_for_request,
        discussion_by_client_id, discussion_by_id, discussions_page_for_request, insert_discussion,
        insert_reply, read_state, read_states_for_user, replies_for_discussion, reply_by_client_id,
        reply_by_id, reply_previews_for_discussions, save_discussion, save_read_state,
        unread_content_counts, users_by_ids as load_users_by_ids,
    },
    request_revision_rows::{
        RequestRevisionWindow, revision_by_id, revision_positions_for_request,
        revision_window_for_request,
    },
    request_rows::insert_request_event_row,
    request_rows::save_request_row,
};
use sea_orm::TransactionTrait;
use std::{collections::BTreeMap, sync::Arc};
use {
    crate::error::PostgresError,
    scope_domain::account::UserAccount,
    scope_domain::requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
        CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
        MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput,
        ReopenRequestDiscussionInput, RequestDiscussion, RequestDiscussionReadState,
        RequestRevision, ResolveRequestDiscussionInput, create_request_discussion,
        create_request_discussion_reply, ensure_request_discussion_transition_allowed,
        mark_request_discussion_read, reopen_and_reply_to_request_discussion,
        reopen_request_discussion, resolve_request_discussion,
    },
};

#[derive(Clone, Debug)]
pub struct RequestDiscussionReadModel {
    pub discussion: RequestDiscussion,
    pub anchor_revision_position: Option<u64>,
    pub reply_count: u64,
    pub latest_replies: Vec<RequestDiscussionReplyReadModel>,
    pub read_through_position: u64,
    pub unread_count: u64,
}

#[derive(Clone, Debug)]
pub struct RequestDiscussionReadBatch {
    pub discussions: Vec<RequestDiscussionReadModel>,
    pub users: BTreeMap<String, UserAccount>,
}

#[derive(Clone, Debug)]
pub struct RequestDiscussionsPageQuery<'a> {
    pub request_id: &'a str,
    pub viewer_user_id: Option<&'a str>,
    pub snapshot_version: u64,
    pub cursor: Option<(u64, String)>,
    pub discussion_id: Option<&'a str>,
    pub anchor_revision_id: Option<&'a str>,
    pub anchor_commit_oid: Option<&'a str>,
    pub include_revision_anchor: bool,
    pub limit: u64,
}

impl RequestStore {
    pub async fn request_revision_window(
        &self,
        request_id: &str,
        selected_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<RequestRevisionWindow, PostgresError> {
        revision_window_for_request(self.db.as_ref(), request_id, selected_revision_id, limit).await
    }

    pub async fn request_discussions_page(
        &self,
        query: RequestDiscussionsPageQuery<'_>,
    ) -> Result<RequestDiscussionReadBatch, PostgresError> {
        let discussions = discussions_page_for_request(
            self.db.as_ref(),
            query.request_id,
            query.snapshot_version,
            query.cursor,
            DiscussionPageFilter {
                discussion_id: query.discussion_id,
                revision_id: query.anchor_revision_id,
                commit_oid: query.anchor_commit_oid,
                include_revision_anchor: query.include_revision_anchor,
            },
            query.limit,
        )
        .await?;
        self.hydrate_discussions(discussions, query.viewer_user_id)
            .await
    }

    pub async fn request_discussion(
        &self,
        request_id: &str,
        discussion_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<(RequestDiscussionReadModel, BTreeMap<String, UserAccount>)>, PostgresError>
    {
        let discussion = match discussion_by_id(self.db.as_ref(), discussion_id).await? {
            Some(discussion) if discussion.request_id == request_id => discussion,
            _ => return Ok(None),
        };
        let mut batch = self
            .hydrate_discussions(vec![discussion], viewer_user_id)
            .await?;
        Ok(batch
            .discussions
            .pop()
            .map(|discussion| (discussion, batch.users)))
    }

    pub async fn changed_request_discussions(
        &self,
        request_id: &str,
        viewer_user_id: Option<&str>,
        after_position: u64,
        limit: u64,
    ) -> Result<RequestDiscussionReadBatch, PostgresError> {
        let discussions =
            changed_discussions_for_request(self.db.as_ref(), request_id, after_position, limit)
                .await?;
        self.hydrate_discussions(discussions, viewer_user_id).await
    }

    async fn hydrate_discussions(
        &self,
        discussions: Vec<RequestDiscussion>,
        viewer_user_id: Option<&str>,
    ) -> Result<RequestDiscussionReadBatch, PostgresError> {
        let revision_positions = match discussions
            .iter()
            .find(|discussion| discussion.anchor.is_some())
        {
            Some(discussion) => {
                revision_positions_for_request(self.db.as_ref(), &discussion.request_id).await?
            }
            None => BTreeMap::new(),
        };
        let ids = discussions
            .iter()
            .map(|discussion| discussion.id.clone())
            .collect::<Vec<_>>();
        let read_states = match viewer_user_id {
            Some(user_id) => read_states_for_user(self.db.as_ref(), &ids, user_id).await?,
            None => BTreeMap::new(),
        };
        let previews = reply_previews_for_discussions(self.db.as_ref(), &ids).await?;
        let unread_counts = match viewer_user_id {
            Some(_) => unread_content_counts(self.db.as_ref(), &discussions, &read_states).await?,
            None => BTreeMap::new(),
        };
        let mut user_ids = discussions
            .iter()
            .flat_map(|discussion| {
                [
                    Some(discussion.author_user_id.clone()),
                    discussion.resolved_by_user_id.clone(),
                ]
            })
            .flatten()
            .collect::<Vec<_>>();
        let mut models = Vec::with_capacity(discussions.len());
        for discussion in discussions {
            let anchor_revision_position = discussion
                .anchor
                .as_ref()
                .map(|anchor| {
                    revision_positions
                        .get(&anchor.revision_id)
                        .copied()
                        .ok_or_else(|| {
                            PostgresError::internal_message(format!(
                                "request discussion anchor references unknown revision {}",
                                anchor.revision_id
                            ))
                        })
                })
                .transpose()?;
            let (reply_count, latest_replies) =
                previews.get(&discussion.id).cloned().unwrap_or_default();
            user_ids.extend(latest_replies.iter().flat_map(|model| {
                [
                    Some(model.reply.author_user_id.clone()),
                    model
                        .reply_to
                        .as_ref()
                        .map(|target| target.author_user_id.clone()),
                ]
                .into_iter()
                .flatten()
            }));
            let read_through_position = read_states
                .get(&discussion.id)
                .map(|state| state.read_through_position)
                .unwrap_or(0);
            let unread_count = unread_counts.get(&discussion.id).copied().unwrap_or(0);
            models.push(RequestDiscussionReadModel {
                discussion,
                anchor_revision_position,
                reply_count,
                latest_replies,
                read_through_position,
                unread_count,
            });
        }
        let users = load_users_by_ids(self.db.as_ref(), user_ids).await?;
        Ok(RequestDiscussionReadBatch {
            discussions: models,
            users,
        })
    }

    pub async fn request_discussion_replies(
        &self,
        discussion_id: &str,
        before_position: Option<u64>,
        limit: u64,
    ) -> Result<
        (
            Vec<RequestDiscussionReplyReadModel>,
            BTreeMap<String, UserAccount>,
        ),
        PostgresError,
    > {
        let replies =
            replies_for_discussion(self.db.as_ref(), discussion_id, before_position, limit).await?;
        let users = load_users_by_ids(
            self.db.as_ref(),
            replies.iter().flat_map(|model| {
                [
                    Some(model.reply.author_user_id.clone()),
                    model
                        .reply_to
                        .as_ref()
                        .map(|target| target.author_user_id.clone()),
                ]
                .into_iter()
                .flatten()
            }),
        )
        .await?;
        Ok((replies, users))
    }

    pub async fn request_discussion_reply(
        &self,
        discussion_id: &str,
        reply_id: &str,
    ) -> Result<
        Option<(
            RequestDiscussionReplyReadModel,
            BTreeMap<String, UserAccount>,
        )>,
        PostgresError,
    > {
        let Some(reply) = reply_by_id(self.db.as_ref(), reply_id).await? else {
            return Ok(None);
        };
        if reply.discussion_id != discussion_id {
            return Ok(None);
        }
        self.request_discussion_reply_read_model(reply)
            .await
            .map(Some)
    }

    pub async fn users_by_ids(
        &self,
        user_ids: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, UserAccount>, PostgresError> {
        load_users_by_ids(self.db.as_ref(), user_ids).await
    }

    pub async fn request_discussion_reply_read_model(
        &self,
        reply: scope_domain::requests::RequestDiscussionReply,
    ) -> Result<
        (
            RequestDiscussionReplyReadModel,
            BTreeMap<String, UserAccount>,
        ),
        PostgresError,
    > {
        let reply_to = match reply.reply_to_reply_id.as_deref() {
            Some(reply_id) => {
                let target = reply_by_id(self.db.as_ref(), reply_id)
                    .await?
                    .ok_or_else(|| {
                        PostgresError::internal_message(format!(
                            "discussion reply references missing reply {reply_id}"
                        ))
                    })?;
                if target.discussion_id != reply.discussion_id || target.position >= reply.position
                {
                    return Err(PostgresError::internal_message(format!(
                        "discussion reply {} has an invalid reply target",
                        reply.id
                    )));
                }
                Some(RequestDiscussionReplyReferenceReadModel {
                    id: target.id,
                    position: target.position,
                    author_user_id: target.author_user_id,
                    body_markdown: target.body_markdown,
                })
            }
            None => None,
        };
        let users = load_users_by_ids(
            self.db.as_ref(),
            [
                Some(reply.author_user_id.clone()),
                reply_to
                    .as_ref()
                    .map(|target| target.author_user_id.clone()),
            ]
            .into_iter()
            .flatten(),
        )
        .await?;
        Ok((RequestDiscussionReplyReadModel { reply, reply_to }, users))
    }

    pub async fn request_revision(
        &self,
        request_id: &str,
        revision_id: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        Ok(revision_by_id(self.db.as_ref(), revision_id)
            .await?
            .filter(|revision| revision.request_id == request_id))
    }

    pub async fn create_request_discussion(
        &self,
        command: CreateRequestDiscussionCommand,
    ) -> Result<CreateRequestDiscussionMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let input = CreateRequestDiscussionInput {
            request_id: command.request_id,
            id: command.id,
            actor_user_id: command.actor_user_id,
            actor_can_participate: policy.permissions.can_open_discussion,
            client_discussion_id: command.client_discussion_id,
            body_markdown: command.body_markdown,
            anchor: command.anchor,
            now_unix: command.now_unix,
        };

        if let Some(discussion) = discussion_by_client_id(
            &tx,
            &input.request_id,
            &input.actor_user_id,
            &input.client_discussion_id,
        )
        .await?
        {
            let state = match read_state(&tx, &discussion.id, &input.actor_user_id).await? {
                Some(state) => state,
                None => {
                    monotonic_read_state(
                        &tx,
                        &discussion,
                        &input.actor_user_id,
                        discussion.opened_position,
                        input.now_unix,
                    )
                    .await?
                }
            };
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CreateRequestDiscussionMutation {
                created: false,
                request,
                discussion,
                read_state: state,
            });
        }

        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut discussions = BTreeMap::new();
        if let Some(existing) = discussion_by_id(&tx, &input.id).await? {
            discussions.insert(existing.id.clone(), existing);
        }
        let mutation = create_request_discussion(&mut requests, &mut discussions, input)?;
        save_request_row(&tx, &mutation.request).await?;
        insert_discussion(&tx, &mutation.discussion).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn create_request_discussion_reply(
        &self,
        command: CreateRequestDiscussionReplyCommand,
    ) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let input = CreateRequestDiscussionReplyInput {
            request_id: command.request_id,
            discussion_id: command.discussion_id,
            id: command.id,
            actor_user_id: command.actor_user_id,
            actor_can_participate: policy.permissions.can_reply_to_discussion,
            client_reply_id: command.client_reply_id,
            body_markdown: command.body_markdown,
            reply_to_reply_id: command.reply_to_reply_id,
            now_unix: command.now_unix,
        };
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == input.request_id)
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;

        if let Some(reply) = reply_by_client_id(
            &tx,
            &input.discussion_id,
            &input.actor_user_id,
            &input.client_reply_id,
        )
        .await?
        {
            let state = monotonic_read_state(
                &tx,
                &discussion,
                &input.actor_user_id,
                reply.position,
                input.now_unix,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CreateRequestDiscussionReplyMutation {
                request,
                discussion,
                reply,
                read_state: state,
                activity_event: None,
            });
        }

        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mut replies = BTreeMap::new();
        if let Some(quoted_id) = input.reply_to_reply_id.as_deref()
            && let Some(reply) = reply_by_id(&tx, quoted_id).await?
        {
            replies.insert(reply.id.clone(), reply);
        }
        if let Some(existing) = reply_by_id(&tx, &input.id).await? {
            replies.insert(existing.id.clone(), existing);
        }
        let mutation =
            create_request_discussion_reply(&mut requests, &mut discussions, &mut replies, input)?;
        save_request_row(&tx, &mutation.request).await?;
        save_discussion(&tx, &mutation.discussion).await?;
        insert_reply(&tx, &mutation.reply).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn transition_request_discussion(
        &self,
        command: TransitionRequestDiscussionCommand,
    ) -> Result<RequestDiscussion, PostgresError> {
        let TransitionRequestDiscussionCommand {
            request_id,
            discussion_id,
            actor_user_id,
            event_id,
            now_unix,
            transition,
        } = command;
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &request_id, &actor_user_id).await?;
        ensure_user_exists(&tx, &actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &actor_user_id).await?;
        if !policy.discussion_visible {
            return Err(PostgresError::not_found("request not found"));
        }
        let discussion = discussion_by_id(&tx, &discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == request_id)
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        let actor_is_maintainer = repo.access.is_maintainer();
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mutation = match transition {
            DiscussionTransition::Resolve => resolve_request_discussion(
                &mut requests,
                &mut discussions,
                ResolveRequestDiscussionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    actor_can_transition: policy.permissions.can_transition_discussion,
                    event_id,
                    now_unix,
                },
            )?,
            DiscussionTransition::Reopen => reopen_request_discussion(
                &mut requests,
                &mut discussions,
                ReopenRequestDiscussionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    actor_can_transition: policy.permissions.can_transition_discussion,
                    event_id,
                    now_unix,
                },
            )?,
        };
        save_request_row(&tx, &mutation.request).await?;
        save_discussion(&tx, &mutation.discussion).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        monotonic_read_state(
            &tx,
            &mutation.discussion,
            &mutation.event.actor_user_id,
            mutation.discussion.last_activity_position,
            now_unix,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation.discussion)
    }

    pub async fn reopen_and_reply_to_request_discussion(
        &self,
        command: ReopenAndReplyToRequestDiscussionCommand,
    ) -> Result<CreateRequestDiscussionReplyMutation, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let actor_is_maintainer = repo.access.is_maintainer();
        let input = ReopenAndReplyToRequestDiscussionInput {
            request_id: command.request_id,
            discussion_id: command.discussion_id,
            reply_id: command.reply_id,
            actor_user_id: command.actor_user_id,
            actor_is_maintainer,
            actor_can_transition: policy.permissions.can_transition_discussion,
            actor_can_participate: policy.permissions.can_reply_to_discussion,
            event_id: command.event_id,
            client_reply_id: command.client_reply_id,
            body_markdown: command.body_markdown,
            reply_to_reply_id: command.reply_to_reply_id,
            now_unix: command.now_unix,
        };
        ensure_request_discussion_transition_allowed(&request, input.actor_can_transition)?;
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == input.request_id)
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        if let Some(reply) = reply_by_client_id(
            &tx,
            &input.discussion_id,
            &input.actor_user_id,
            &input.client_reply_id,
        )
        .await?
        {
            let state = monotonic_read_state(
                &tx,
                &discussion,
                &input.actor_user_id,
                reply.position,
                input.now_unix,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(CreateRequestDiscussionReplyMutation {
                request,
                discussion,
                reply,
                read_state: state,
                activity_event: None,
            });
        }
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mut replies = BTreeMap::new();
        if let Some(quoted_id) = input.reply_to_reply_id.as_deref()
            && let Some(reply) = reply_by_id(&tx, quoted_id).await?
        {
            replies.insert(reply.id.clone(), reply);
        }
        let mutation = reopen_and_reply_to_request_discussion(
            &mut requests,
            &mut discussions,
            &mut replies,
            input,
        )?;
        save_request_row(&tx, &mutation.request).await?;
        save_discussion(&tx, &mutation.discussion).await?;
        insert_reply(&tx, &mutation.reply).await?;
        save_read_state(&tx, &mutation.read_state).await?;
        if let Some(event) = &mutation.activity_event {
            insert_request_event_row(&tx, event).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn mark_request_discussion_read(
        &self,
        input: MarkRequestDiscussionReadInput,
    ) -> Result<RequestDiscussionReadState, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(PostgresError::internal)?;
        ensure_user_exists(&tx, &input.user_id).await?;
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request discussion not found"))?;
        let (repo, request) =
            lock_request_repository(&tx, &discussion.request_id, &input.user_id).await?;
        if !request_policy_for_user(&tx, &repo, &request, &input.user_id)
            .await?
            .discussion_visible
        {
            return Err(PostgresError::not_found("request not found"));
        }
        let discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mut read_states = BTreeMap::new();
        if let Some(state) = read_state(&tx, &input.discussion_id, &input.user_id).await? {
            read_states.insert((input.discussion_id.clone(), input.user_id.clone()), state);
        }
        let state = mark_request_discussion_read(&discussions, &mut read_states, input)?;
        save_read_state(&tx, &state).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(state)
    }
}

async fn monotonic_read_state<C>(
    conn: &C,
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> Result<RequestDiscussionReadState, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let mut states = BTreeMap::new();
    if let Some(state) = read_state(conn, &discussion.id, user_id).await? {
        states.insert((discussion.id.clone(), user_id.to_string()), state);
    }
    let discussions = BTreeMap::from([(discussion.id.clone(), discussion.clone())]);
    let state = mark_request_discussion_read(
        &discussions,
        &mut states,
        MarkRequestDiscussionReadInput {
            discussion_id: discussion.id.clone(),
            user_id: user_id.to_string(),
            through_position: position,
            now_unix,
        },
    )?;
    save_read_state(conn, &state).await?;
    Ok(state)
}
