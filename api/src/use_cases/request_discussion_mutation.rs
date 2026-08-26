use crate::{
    auth::scope::principal_for_user_id,
    error::ApiError,
    persistence::unix_now,
    product_analytics::ProductEvent,
    repo_access::{ensure_repo_read, find_repo},
    state::AppState,
};
use scope_domain::{
    account::UserAccount,
    repository::{Repository, access::RepositoryAccess},
    requests::{
        CreateRequestDiscussionInput, CreateRequestDiscussionReplyInput,
        MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput, Request,
        RequestDiscussionReply, RequestViewer, request_actor_role, request_policy,
    },
};
use scope_postgres::db::RequestDiscussionReadModel;
use std::collections::{BTreeMap, BTreeSet};

mod anchor;

pub(crate) struct DiscussionAnchorInput {
    pub(crate) revision_id: String,
    pub(crate) commit_oid: Option<String>,
    pub(crate) path: Option<String>,
}

pub(crate) struct CreateDiscussionCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_discussion_id: String,
    pub(crate) body_markdown: String,
    pub(crate) anchor: Option<DiscussionAnchorInput>,
}

pub(crate) struct CreateReplyCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_reply_id: String,
    pub(crate) body_markdown: String,
    pub(crate) reply_to_reply_id: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum DiscussionTransition {
    Resolve,
    Reopen,
}

pub(crate) struct TransitionDiscussionCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) transition: DiscussionTransition,
}

pub(crate) struct ReopenAndReplyCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) client_reply_id: String,
    pub(crate) body_markdown: String,
    pub(crate) reply_to_reply_id: Option<String>,
}

pub(crate) struct MarkDiscussionReadCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) discussion_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) through_position: u64,
}

pub(crate) struct DiscussionMutationResult {
    pub(crate) discussion: RequestDiscussionReadModel,
    pub(crate) users: BTreeMap<String, UserAccount>,
    pub(crate) visible_anchor_commits: BTreeSet<(String, String)>,
}

pub(crate) struct ReplyMutationResult {
    pub(crate) discussion: DiscussionMutationResult,
    pub(crate) reply: RequestDiscussionReply,
    pub(crate) reply_users: BTreeMap<String, UserAccount>,
    pub(crate) child_reply_count: u64,
}

pub(crate) struct MarkDiscussionReadResult {
    pub(crate) read_through_position: u64,
}

pub(super) struct MutationContext {
    pub(super) repo: Repository,
    pub(super) access: RepositoryAccess,
    pub(super) request: Request,
}

pub(crate) async fn create_discussion(
    state: &AppState,
    command: CreateDiscussionCommand,
) -> Result<DiscussionMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let anchor = match command.anchor {
        Some(anchor) => Some(
            anchor::validate(state, &command.owner, &command.repo_name, &context, anchor).await?,
        ),
        None => None,
    };
    let mutation = state
        .metadata
        .requests()
        .create_request_discussion(CreateRequestDiscussionInput {
            request_id: context.request.id.clone(),
            id: random_id("discussion")?,
            actor_user_id: command.actor_user_id.clone(),
            actor_can_participate: false,
            client_discussion_id: command.client_discussion_id,
            body_markdown: command.body_markdown,
            anchor,
            now_unix: unix_now()?,
        })
        .await?;
    if mutation.created {
        state
            .product_analytics
            .capture(ProductEvent::discussion_created(
                &command.actor_user_id,
                context.request.audience,
                request_actor_role(context.access),
                mutation.discussion.anchor.is_some(),
            ));
    }
    let discussion_id = mutation.discussion.id.clone();
    let through_position = mutation.discussion.last_activity_position;
    let result = load_discussion_result(
        state,
        &command.owner,
        &command.repo_name,
        &context,
        &discussion_id,
        &command.actor_user_id,
    )
    .await?;
    publish_timeline_change(state, &context, discussion_id, through_position).await;
    Ok(result)
}

pub(crate) async fn create_reply(
    state: &AppState,
    command: CreateReplyCommand,
) -> Result<ReplyMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let mutation = state
        .metadata
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyInput {
            request_id: context.request.id.clone(),
            discussion_id: command.discussion_id,
            id: random_id("discussion_reply")?,
            actor_user_id: command.actor_user_id.clone(),
            actor_can_participate: false,
            client_reply_id: command.client_reply_id,
            body_markdown: command.body_markdown,
            reply_to_reply_id: command.reply_to_reply_id,
            now_unix: unix_now()?,
        })
        .await?;
    reply_mutation_result(
        state,
        &command.owner,
        &command.repo_name,
        &context,
        mutation.discussion.id,
        mutation.reply,
        &command.actor_user_id,
    )
    .await
}

pub(crate) async fn transition_discussion(
    state: &AppState,
    command: TransitionDiscussionCommand,
) -> Result<DiscussionMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let discussion = match command.transition {
        DiscussionTransition::Resolve => {
            state
                .metadata
                .requests()
                .resolve_request_discussion(
                    context.request.id.clone(),
                    command.discussion_id.clone(),
                    command.actor_user_id.clone(),
                    random_id("event_request_discussion_resolved")?,
                    unix_now()?,
                )
                .await?
        }
        DiscussionTransition::Reopen => {
            state
                .metadata
                .requests()
                .reopen_request_discussion(
                    context.request.id.clone(),
                    command.discussion_id.clone(),
                    command.actor_user_id.clone(),
                    random_id("event_request_discussion_reopened")?,
                    unix_now()?,
                )
                .await?
        }
    };
    if matches!(command.transition, DiscussionTransition::Resolve) {
        state
            .product_analytics
            .capture(ProductEvent::discussion_resolved(
                &command.actor_user_id,
                context.request.audience,
                request_actor_role(context.access),
            ));
    }
    let through_position = discussion.last_activity_position;
    let result = load_discussion_result(
        state,
        &command.owner,
        &command.repo_name,
        &context,
        &command.discussion_id,
        &command.actor_user_id,
    )
    .await?;
    publish_timeline_change(state, &context, command.discussion_id, through_position).await;
    Ok(result)
}

pub(crate) async fn reopen_and_reply(
    state: &AppState,
    command: ReopenAndReplyCommand,
) -> Result<ReplyMutationResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    let mutation = state
        .metadata
        .requests()
        .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionInput {
            request_id: context.request.id.clone(),
            discussion_id: command.discussion_id,
            reply_id: random_id("discussion_reply")?,
            actor_user_id: command.actor_user_id.clone(),
            actor_is_maintainer: false,
            actor_can_transition: false,
            actor_can_participate: false,
            event_id: random_id("event_request_discussion_reopened")?,
            client_reply_id: command.client_reply_id,
            body_markdown: command.body_markdown,
            reply_to_reply_id: command.reply_to_reply_id,
            now_unix: unix_now()?,
        })
        .await?;
    reply_mutation_result(
        state,
        &command.owner,
        &command.repo_name,
        &context,
        mutation.discussion.id,
        mutation.reply,
        &command.actor_user_id,
    )
    .await
}

pub(crate) async fn mark_read(
    state: &AppState,
    command: MarkDiscussionReadCommand,
) -> Result<MarkDiscussionReadResult, ApiError> {
    let context = mutation_context(
        state,
        &command.owner,
        &command.repo_name,
        &command.request_id,
        &command.actor_user_id,
    )
    .await?;
    ensure_discussion_in_request(state, &context.request.id, &command.discussion_id).await?;
    let read_state = state
        .metadata
        .requests()
        .mark_request_discussion_read(MarkRequestDiscussionReadInput {
            discussion_id: command.discussion_id,
            user_id: command.actor_user_id,
            through_position: command.through_position,
            now_unix: unix_now()?,
        })
        .await?;
    Ok(MarkDiscussionReadResult {
        read_through_position: read_state.read_through_position,
    })
}

async fn mutation_context(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_id: &str,
    actor_user_id: &str,
) -> Result<MutationContext, ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let principal = principal_for_user_id(&repo, actor_user_id);
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let request = state
        .metadata
        .requests()
        .request_by_id(request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, actor_user_id)
        .await?;
    if request.repo_id != repo.record.id
        || !request_policy(
            &request,
            RequestViewer::new(access, Some(actor_user_id), is_invitee),
        )
        .exact_visible
    {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(MutationContext {
        repo,
        access,
        request,
    })
}

async fn reply_mutation_result(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    context: &MutationContext,
    discussion_id: String,
    reply: RequestDiscussionReply,
    actor_user_id: &str,
) -> Result<ReplyMutationResult, ApiError> {
    let discussion = load_discussion_result(
        state,
        owner,
        repo_name,
        context,
        &discussion_id,
        actor_user_id,
    )
    .await?;
    let reply_users = state
        .metadata
        .requests()
        .users_by_ids([reply.author_user_id.clone()])
        .await?;
    let child_reply_count = state
        .metadata
        .requests()
        .request_discussion_reply_child_count(&reply.id)
        .await?;
    publish_timeline_change(state, context, discussion_id, reply.position).await;
    Ok(ReplyMutationResult {
        discussion,
        reply,
        reply_users,
        child_reply_count,
    })
}

async fn load_discussion_result(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    context: &MutationContext,
    discussion_id: &str,
    viewer_user_id: &str,
) -> Result<DiscussionMutationResult, ApiError> {
    let (discussion, users) = state
        .metadata
        .requests()
        .request_discussion(&context.request.id, discussion_id, Some(viewer_user_id))
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    let visible_anchor_commits = anchor::visible_commits(
        state,
        owner,
        repo_name,
        context,
        discussion.discussion.anchor.as_ref(),
    )
    .await;
    Ok(DiscussionMutationResult {
        discussion,
        users,
        visible_anchor_commits,
    })
}

async fn publish_timeline_change(
    state: &AppState,
    context: &MutationContext,
    discussion_id: String,
    through_position: u64,
) {
    state
        .publish_request_timeline_change(
            &context.repo.record.id,
            context.request.id.clone(),
            discussion_id,
            through_position,
            context.request.audience,
        )
        .await;
}

async fn ensure_discussion_in_request(
    state: &AppState,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), ApiError> {
    state
        .metadata
        .requests()
        .request_discussion(request_id, discussion_id, None)
        .await?
        .ok_or_else(|| ApiError::not_found("request discussion not found"))?;
    Ok(())
}

fn random_id(prefix: &str) -> Result<String, ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!("failed to create {prefix} id: {error}"))
    })?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}
