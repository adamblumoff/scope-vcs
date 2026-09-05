//! Actor identity and mutation intent. Locked persistence facts supply domain capabilities.

use scope_domain::requests::RequestDiscussionAnchor;

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionCommand {
    pub request_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub client_discussion_id: String,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionReplyCommand {
    pub request_id: String,
    pub discussion_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ReopenAndReplyToRequestDiscussionCommand {
    pub request_id: String,
    pub discussion_id: String,
    pub reply_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum DiscussionTransition {
    Resolve,
    Reopen,
}

#[derive(Clone, Debug)]
pub struct TransitionRequestDiscussionCommand {
    pub request_id: String,
    pub discussion_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub now_unix: u64,
    pub transition: DiscussionTransition,
}
