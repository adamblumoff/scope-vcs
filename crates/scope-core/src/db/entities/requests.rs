use super::*;
use crate::domain::requests::{
    CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestActorRole, RequestAudience,
    RequestDiscussion, RequestDiscussionReadState, RequestDiscussionReply, RequestDiscussionStatus,
    RequestDisposition, RequestEvent, RequestEventKind, RequestEventPayload, RequestSettlement,
    RequestState, UserCreditAccount,
};
use crate::domain::store::SourceBlob;

pub mod request {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_requests")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub repo_id: String,
        pub name: String,
        pub author_user_id: String,
        pub author_role: String,
        pub audience: String,
        pub base_main_oid: String,
        pub head_oid: String,
        pub git_snapshot: Option<Json>,
        pub title: String,
        pub description_markdown: String,
        pub state: String,
        pub activity_version: i64,
        pub stake_credits: i32,
        pub disposition: Option<String>,
        pub settlement: Option<Json>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub resolved_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(request: &Request) -> Result<Self, ApiError> {
            Ok(Self {
                id: request.id.clone(),
                repo_id: request.repo_id.clone(),
                name: request.name.clone(),
                author_user_id: request.author_user_id.clone(),
                author_role: encode_enum(request.author_role)?,
                audience: encode_enum(request.audience)?,
                base_main_oid: request.base_main_oid.clone(),
                head_oid: request.head_oid.clone(),
                git_snapshot: request.git_snapshot.as_ref().map(encode_json).transpose()?,
                title: request.title.clone(),
                description_markdown: request.description_markdown.clone(),
                state: encode_enum(request.state)?,
                activity_version: u64_to_i64(request.activity_version, "request activity version")?,
                stake_credits: u32_to_i32(request.stake_credits, "request stake credits")?,
                disposition: request.disposition.map(encode_enum).transpose()?,
                settlement: request.settlement.as_ref().map(encode_json).transpose()?,
                created_at_unix: u64_to_i64(request.created_at_unix, "request creation time")?,
                updated_at_unix: u64_to_i64(request.updated_at_unix, "request update time")?,
                resolved_at_unix: request
                    .resolved_at_unix
                    .map(|value| u64_to_i64(value, "request resolution time"))
                    .transpose()?,
            })
        }

        pub fn try_into_domain(self) -> Result<Request, ApiError> {
            Ok(Request {
                id: self.id,
                repo_id: self.repo_id,
                name: self.name,
                author_user_id: self.author_user_id,
                author_role: decode_enum::<RequestActorRole>(self.author_role)?,
                audience: decode_enum::<RequestAudience>(self.audience)?,
                base_main_oid: self.base_main_oid,
                head_oid: self.head_oid,
                git_snapshot: self
                    .git_snapshot
                    .map(decode_json::<SourceBlob>)
                    .transpose()?,
                title: self.title,
                description_markdown: self.description_markdown,
                state: decode_enum::<RequestState>(self.state)?,
                activity_version: i64_to_u64(self.activity_version, "request activity version")?,
                stake_credits: i32_to_u32(self.stake_credits, "request stake credits")?,
                disposition: self
                    .disposition
                    .map(decode_enum::<RequestDisposition>)
                    .transpose()?,
                settlement: self
                    .settlement
                    .map(decode_json::<RequestSettlement>)
                    .transpose()?,
                created_at_unix: i64_to_u64(self.created_at_unix, "request creation time")?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "request update time")?,
                resolved_at_unix: self
                    .resolved_at_unix
                    .map(|value| i64_to_u64(value, "request resolution time"))
                    .transpose()?,
            })
        }
    }
}

pub mod request_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub request_id: String,
        pub actor_user_id: String,
        pub kind: String,
        pub position: i64,
        pub payload: Json,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(event: &RequestEvent) -> Result<Self, ApiError> {
            Ok(Self {
                id: event.id.clone(),
                request_id: event.request_id.clone(),
                actor_user_id: event.actor_user_id.clone(),
                kind: encode_enum(event.kind)?,
                position: u64_to_i64(event.position, "request event position")?,
                payload: encode_json(&event.payload)?,
                created_at_unix: u64_to_i64(event.created_at_unix, "request event creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestEvent, ApiError> {
            Ok(RequestEvent {
                id: self.id,
                request_id: self.request_id,
                actor_user_id: self.actor_user_id,
                kind: decode_enum::<RequestEventKind>(self.kind)?,
                position: i64_to_u64(self.position, "request event position")?,
                payload: decode_json::<RequestEventPayload>(self.payload)?,
                created_at_unix: i64_to_u64(self.created_at_unix, "request event creation time")?,
            })
        }
    }
}

pub mod request_discussion {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub request_id: String,
        pub opened_position: i64,
        pub last_activity_position: i64,
        pub author_user_id: String,
        pub body_markdown: String,
        pub status: String,
        pub client_discussion_id: String,
        pub created_at_unix: i64,
        pub resolved_at_unix: Option<i64>,
        pub resolved_by_user_id: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussion) -> Result<Self, ApiError> {
            Ok(Self {
                id: value.id.clone(),
                request_id: value.request_id.clone(),
                opened_position: u64_to_i64(value.opened_position, "discussion opened position")?,
                last_activity_position: u64_to_i64(
                    value.last_activity_position,
                    "discussion last activity position",
                )?,
                author_user_id: value.author_user_id.clone(),
                body_markdown: value.body_markdown.clone(),
                status: encode_enum(value.status)?,
                client_discussion_id: value.client_discussion_id.clone(),
                created_at_unix: u64_to_i64(value.created_at_unix, "discussion creation time")?,
                resolved_at_unix: value
                    .resolved_at_unix
                    .map(|time| u64_to_i64(time, "discussion resolution time"))
                    .transpose()?,
                resolved_by_user_id: value.resolved_by_user_id.clone(),
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussion, ApiError> {
            Ok(RequestDiscussion {
                id: self.id,
                request_id: self.request_id,
                opened_position: i64_to_u64(self.opened_position, "discussion opened position")?,
                last_activity_position: i64_to_u64(
                    self.last_activity_position,
                    "discussion last activity position",
                )?,
                author_user_id: self.author_user_id,
                body_markdown: self.body_markdown,
                status: decode_enum::<RequestDiscussionStatus>(self.status)?,
                client_discussion_id: self.client_discussion_id,
                created_at_unix: i64_to_u64(self.created_at_unix, "discussion creation time")?,
                resolved_at_unix: self
                    .resolved_at_unix
                    .map(|time| i64_to_u64(time, "discussion resolution time"))
                    .transpose()?,
                resolved_by_user_id: self.resolved_by_user_id,
            })
        }
    }
}

pub mod request_discussion_reply {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussion_replies")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub discussion_id: String,
        pub position: i64,
        pub author_user_id: String,
        pub body_markdown: String,
        pub reply_to_reply_id: Option<String>,
        pub client_reply_id: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussionReply) -> Result<Self, ApiError> {
            Ok(Self {
                id: value.id.clone(),
                discussion_id: value.discussion_id.clone(),
                position: u64_to_i64(value.position, "discussion reply position")?,
                author_user_id: value.author_user_id.clone(),
                body_markdown: value.body_markdown.clone(),
                reply_to_reply_id: value.reply_to_reply_id.clone(),
                client_reply_id: value.client_reply_id.clone(),
                created_at_unix: u64_to_i64(
                    value.created_at_unix,
                    "discussion reply creation time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussionReply, ApiError> {
            Ok(RequestDiscussionReply {
                id: self.id,
                discussion_id: self.discussion_id,
                position: i64_to_u64(self.position, "discussion reply position")?,
                author_user_id: self.author_user_id,
                body_markdown: self.body_markdown,
                reply_to_reply_id: self.reply_to_reply_id,
                client_reply_id: self.client_reply_id,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "discussion reply creation time",
                )?,
            })
        }
    }
}

pub mod request_discussion_read_state {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussion_read_states")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub discussion_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub read_through_position: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussionReadState) -> Result<Self, ApiError> {
            Ok(Self {
                discussion_id: value.discussion_id.clone(),
                user_id: value.user_id.clone(),
                read_through_position: u64_to_i64(
                    value.read_through_position,
                    "discussion read position",
                )?,
                updated_at_unix: u64_to_i64(value.updated_at_unix, "discussion read time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussionReadState, ApiError> {
            Ok(RequestDiscussionReadState {
                discussion_id: self.discussion_id,
                user_id: self.user_id,
                read_through_position: i64_to_u64(
                    self.read_through_position,
                    "discussion read position",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "discussion read time")?,
            })
        }
    }
}

pub mod user_credit_account {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_user_credit_accounts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub balance_credits: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(account: &UserCreditAccount) -> Result<Self, ApiError> {
            Ok(Self {
                user_id: account.user_id.clone(),
                balance_credits: u32_to_i32(account.balance_credits, "user credit balance")?,
            })
        }

        pub fn try_into_domain(self) -> Result<UserCreditAccount, ApiError> {
            Ok(UserCreditAccount {
                user_id: self.user_id,
                balance_credits: i32_to_u32(self.balance_credits, "user credit balance")?,
            })
        }
    }
}

pub mod credit_ledger_entry {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_credit_ledger_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub user_id: String,
        pub request_id: Option<String>,
        pub kind: String,
        pub amount_credits: i32,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(entry: &CreditLedgerEntry) -> Result<Self, ApiError> {
            Ok(Self {
                id: entry.id.clone(),
                user_id: entry.user_id.clone(),
                request_id: entry.request_id.clone(),
                kind: encode_enum(entry.kind)?,
                amount_credits: entry.amount_credits,
                created_at_unix: u64_to_i64(
                    entry.created_at_unix,
                    "credit ledger entry creation time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<CreditLedgerEntry, ApiError> {
            Ok(CreditLedgerEntry {
                id: self.id,
                user_id: self.user_id,
                request_id: self.request_id,
                kind: decode_enum::<CreditLedgerEntryKind>(self.kind)?,
                amount_credits: self.amount_credits,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "credit ledger entry creation time",
                )?,
            })
        }
    }
}
