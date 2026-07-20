use super::{
    MetadataStore, acquire_aggregate_lock,
    request_access::{ensure_request_collaborator, ensure_user_exists, repo_by_id},
    request_change_block_rows::{change_block_by_id, change_blocks_by_ids},
    request_discussion_rows::{
        changed_discussions_for_request, discussion_by_client_id, discussion_by_id,
        discussions_page_for_request, insert_discussion, insert_reply, read_state,
        read_states_for_user, replies_for_discussion, reply_by_client_id, reply_by_id,
        reply_previews_for_discussions, save_discussion, save_read_state, unread_content_counts,
        users_by_ids as load_users_by_ids,
    },
    request_rows::insert_request_event_row,
    request_rows::{request_by_id, save_request_row},
};
use crate::{
    domain::requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
        CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
        MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput,
        ReopenRequestDiscussionInput, RequestChangeBlock, RequestDiscussion,
        RequestDiscussionReadState, RequestDiscussionReply, RequestDiscussionStatus,
        RequestDiscussionSubject, ResolveRequestDiscussionInput, create_request_discussion,
        create_request_discussion_reply, mark_request_discussion_read,
        reopen_and_reply_to_request_discussion, reopen_request_discussion,
        resolve_request_discussion,
    },
    domain::store::UserAccount,
    error::ApiError,
};
use sea_orm::TransactionTrait;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct RequestDiscussionReadModel {
    pub discussion: RequestDiscussion,
    pub change_block: Option<RequestChangeBlock>,
    pub reply_count: u64,
    pub latest_replies: Vec<RequestDiscussionReply>,
    pub unread_count: u64,
    pub sort_position: u64,
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
    pub status: Option<RequestDiscussionStatus>,
    pub recent: bool,
    pub snapshot_version: u64,
    pub cursor: Option<(u64, String)>,
    pub limit: u64,
}

impl MetadataStore {
    pub async fn request_discussions_page(
        &self,
        query: RequestDiscussionsPageQuery<'_>,
    ) -> Result<RequestDiscussionReadBatch, ApiError> {
        let page_rows = discussions_page_for_request(
            self.db.as_ref(),
            query.request_id,
            query.status,
            query.recent,
            query.snapshot_version,
            query.cursor,
            query.limit,
        )
        .await?;
        let sort_positions = page_rows
            .iter()
            .map(|(discussion, position)| (discussion.id.clone(), *position))
            .collect::<BTreeMap<_, _>>();
        let discussions = page_rows
            .into_iter()
            .map(|(discussion, _)| discussion)
            .collect();
        let mut batch = self
            .hydrate_discussions(discussions, query.viewer_user_id)
            .await?;
        for model in &mut batch.discussions {
            model.sort_position = sort_positions
                .get(&model.discussion.id)
                .copied()
                .ok_or_else(|| ApiError::internal_message("discussion sort position missing"))?;
        }
        Ok(batch)
    }

    pub async fn request_discussion(
        &self,
        request_id: &str,
        discussion_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<(RequestDiscussionReadModel, BTreeMap<String, UserAccount>)>, ApiError> {
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
    ) -> Result<RequestDiscussionReadBatch, ApiError> {
        let discussions =
            changed_discussions_for_request(self.db.as_ref(), request_id, after_position, limit)
                .await?;
        self.hydrate_discussions(discussions, viewer_user_id).await
    }

    async fn hydrate_discussions(
        &self,
        discussions: Vec<RequestDiscussion>,
        viewer_user_id: Option<&str>,
    ) -> Result<RequestDiscussionReadBatch, ApiError> {
        let ids = discussions
            .iter()
            .map(|discussion| discussion.id.clone())
            .collect::<Vec<_>>();
        let read_states = match viewer_user_id {
            Some(user_id) => read_states_for_user(self.db.as_ref(), &ids, user_id).await?,
            None => BTreeMap::new(),
        };
        let previews = reply_previews_for_discussions(self.db.as_ref(), &ids).await?;
        let change_block_ids = discussions
            .iter()
            .filter_map(|discussion| match &discussion.subject {
                RequestDiscussionSubject::Comment => None,
                RequestDiscussionSubject::ChangeBlock { change_block_id } => {
                    Some(change_block_id.clone())
                }
            })
            .collect::<Vec<_>>();
        let change_blocks = change_blocks_by_ids(self.db.as_ref(), &change_block_ids).await?;
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
            let (reply_count, latest_replies) =
                previews.get(&discussion.id).cloned().unwrap_or_default();
            user_ids.extend(
                latest_replies
                    .iter()
                    .map(|reply| reply.author_user_id.clone()),
            );
            let unread_count = unread_counts.get(&discussion.id).copied().unwrap_or(0);
            models.push(RequestDiscussionReadModel {
                change_block: match &discussion.subject {
                    RequestDiscussionSubject::Comment => None,
                    RequestDiscussionSubject::ChangeBlock { change_block_id } => {
                        Some(change_blocks.get(change_block_id).cloned().ok_or_else(|| {
                            ApiError::internal_message("request change block subject is missing")
                        })?)
                    }
                },
                sort_position: discussion.last_activity_position,
                discussion,
                reply_count,
                latest_replies,
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
    ) -> Result<(Vec<RequestDiscussionReply>, BTreeMap<String, UserAccount>), ApiError> {
        let replies =
            replies_for_discussion(self.db.as_ref(), discussion_id, before_position, limit).await?;
        let users = load_users_by_ids(
            self.db.as_ref(),
            replies.iter().map(|reply| reply.author_user_id.clone()),
        )
        .await?;
        Ok((replies, users))
    }

    pub async fn users_by_ids(
        &self,
        user_ids: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, UserAccount>, ApiError> {
        load_users_by_ids(self.db.as_ref(), user_ids).await
    }

    pub async fn request_change_block(
        &self,
        request_id: &str,
        block_id: &str,
    ) -> Result<Option<RequestChangeBlock>, ApiError> {
        Ok(change_block_by_id(self.db.as_ref(), block_id)
            .await?
            .filter(|block| block.request_id == request_id))
    }

    pub async fn create_request_discussion(
        &self,
        mut input: CreateRequestDiscussionInput,
    ) -> Result<CreateRequestDiscussionMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        ensure_request_collaborator(&repo, &request, &input.actor_user_id)?;
        input.actor_can_participate = true;

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
            tx.commit().await.map_err(ApiError::internal)?;
            return Ok(CreateRequestDiscussionMutation {
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
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn create_request_discussion_reply(
        &self,
        mut input: CreateRequestDiscussionReplyInput,
    ) -> Result<CreateRequestDiscussionReplyMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        ensure_request_collaborator(&repo, &request, &input.actor_user_id)?;
        input.actor_can_participate = true;
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == input.request_id)
            .ok_or_else(|| ApiError::not_found("request discussion not found"))?;

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
            tx.commit().await.map_err(ApiError::internal)?;
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
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn resolve_request_discussion(
        &self,
        request_id: String,
        discussion_id: String,
        actor_user_id: String,
        event_id: String,
        now_unix: u64,
    ) -> Result<RequestDiscussion, ApiError> {
        self.transition_request_discussion(
            request_id,
            discussion_id,
            actor_user_id,
            event_id,
            now_unix,
            true,
        )
        .await
    }

    pub async fn reopen_request_discussion(
        &self,
        request_id: String,
        discussion_id: String,
        actor_user_id: String,
        event_id: String,
        now_unix: u64,
    ) -> Result<RequestDiscussion, ApiError> {
        self.transition_request_discussion(
            request_id,
            discussion_id,
            actor_user_id,
            event_id,
            now_unix,
            false,
        )
        .await
    }

    async fn transition_request_discussion(
        &self,
        request_id: String,
        discussion_id: String,
        actor_user_id: String,
        event_id: String,
        now_unix: u64,
        resolve: bool,
    ) -> Result<RequestDiscussion, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &request_id).await?;
        ensure_user_exists(&tx, &actor_user_id).await?;
        let request = request_by_id(&tx, &request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        let discussion = discussion_by_id(&tx, &discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == request_id)
            .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
        let actor_is_maintainer = repo.is_maintainer_user_id(&actor_user_id);
        let mut requests = BTreeMap::from([(request.id.clone(), request)]);
        let mut discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mutation = if resolve {
            resolve_request_discussion(
                &mut requests,
                &mut discussions,
                ResolveRequestDiscussionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    event_id,
                    now_unix,
                },
            )?
        } else {
            reopen_request_discussion(
                &mut requests,
                &mut discussions,
                ReopenRequestDiscussionInput {
                    request_id,
                    discussion_id,
                    actor_user_id,
                    actor_is_maintainer,
                    event_id,
                    now_unix,
                },
            )?
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
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation.discussion)
    }

    pub async fn reopen_and_reply_to_request_discussion(
        &self,
        mut input: ReopenAndReplyToRequestDiscussionInput,
    ) -> Result<CreateRequestDiscussionReplyMutation, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;
        ensure_user_exists(&tx, &input.actor_user_id).await?;
        let request = request_by_id(&tx, &input.request_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request not found"))?;
        let repo = repo_by_id(&tx, &request.repo_id).await?;
        ensure_request_collaborator(&repo, &request, &input.actor_user_id)?;
        input.actor_can_participate = true;
        input.actor_is_maintainer = repo.is_maintainer_user_id(&input.actor_user_id);
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .filter(|discussion| discussion.request_id == input.request_id)
            .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
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
            tx.commit().await.map_err(ApiError::internal)?;
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
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(mutation)
    }

    pub async fn mark_request_discussion_read(
        &self,
        input: MarkRequestDiscussionReadInput,
    ) -> Result<RequestDiscussionReadState, ApiError> {
        let db = Arc::clone(&self.db);
        let tx = db.as_ref().begin().await.map_err(ApiError::internal)?;
        ensure_user_exists(&tx, &input.user_id).await?;
        let discussion = discussion_by_id(&tx, &input.discussion_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
        acquire_aggregate_lock(&tx, "request", &discussion.request_id).await?;
        let discussions = BTreeMap::from([(discussion.id.clone(), discussion)]);
        let mut read_states = BTreeMap::new();
        if let Some(state) = read_state(&tx, &input.discussion_id, &input.user_id).await? {
            read_states.insert((input.discussion_id.clone(), input.user_id.clone()), state);
        }
        let state = mark_request_discussion_read(&discussions, &mut read_states, input)?;
        save_read_state(&tx, &state).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(state)
    }
}

async fn monotonic_read_state<C>(
    conn: &C,
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> Result<RequestDiscussionReadState, ApiError>
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
