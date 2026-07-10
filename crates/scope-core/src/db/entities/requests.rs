use super::*;
use crate::domain::requests::{
    CreditLedgerEntry, CreditLedgerEntryKind, Request, RequestActorRole, RequestBaseAudience,
    RequestDisposition, RequestEvent, RequestEventKind, RequestSettlement, RequestState,
    UserCreditAccount,
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
        pub author_user_id: String,
        pub editor_user_ids: Json,
        pub author_role: String,
        pub base_audience: String,
        pub target_branch: String,
        pub request_ref: String,
        pub base_main_oid: String,
        pub head_oid: String,
        pub git_snapshot: Option<Json>,
        pub title: String,
        pub state: String,
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
                author_user_id: request.author_user_id.clone(),
                editor_user_ids: encode_json(&request.editor_user_ids)?,
                author_role: encode_enum(request.author_role)?,
                base_audience: encode_enum(request.base_audience)?,
                target_branch: request.target_branch.clone(),
                request_ref: request.request_ref.clone(),
                base_main_oid: request.base_main_oid.clone(),
                head_oid: request.head_oid.clone(),
                git_snapshot: request.git_snapshot.as_ref().map(encode_json).transpose()?,
                title: request.title.clone(),
                state: encode_enum(request.state)?,
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
                author_user_id: self.author_user_id,
                editor_user_ids: decode_json::<std::collections::BTreeSet<String>>(
                    self.editor_user_ids,
                )?,
                author_role: decode_enum::<RequestActorRole>(self.author_role)?,
                base_audience: decode_enum::<RequestBaseAudience>(self.base_audience)?,
                target_branch: self.target_branch,
                request_ref: self.request_ref,
                base_main_oid: self.base_main_oid,
                head_oid: self.head_oid,
                git_snapshot: self
                    .git_snapshot
                    .map(decode_json::<SourceBlob>)
                    .transpose()?,
                title: self.title,
                state: decode_enum::<RequestState>(self.state)?,
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
        pub body: Option<String>,
        pub old_head_oid: Option<String>,
        pub new_head_oid: Option<String>,
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
                body: event.body.clone(),
                old_head_oid: event.old_head_oid.clone(),
                new_head_oid: event.new_head_oid.clone(),
                created_at_unix: u64_to_i64(event.created_at_unix, "request event creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestEvent, ApiError> {
            Ok(RequestEvent {
                id: self.id,
                request_id: self.request_id,
                actor_user_id: self.actor_user_id,
                kind: decode_enum::<RequestEventKind>(self.kind)?,
                body: self.body,
                old_head_oid: self.old_head_oid,
                new_head_oid: self.new_head_oid,
                created_at_unix: i64_to_u64(self.created_at_unix, "request event creation time")?,
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
