use super::*;

pub mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub handle: String,
        #[sea_orm(unique)]
        pub email: String,
        pub email_verified: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(user: &UserAccount) -> Self {
            Self {
                id: user.id.clone(),
                handle: user.handle.clone(),
                email: user.email.clone(),
                email_verified: user.email_verified,
            }
        }

        pub fn try_into_domain(self) -> Result<UserAccount, ApiError> {
            Ok(UserAccount {
                id: self.id,
                handle: self.handle,
                email: self.email,
                email_verified: self.email_verified,
            })
        }
    }
}
pub mod auth_identity {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_auth_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub provider: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub subject: String,
        pub user_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub mod cli_device_login {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_device_logins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub device_code_hash: String,
        #[sea_orm(unique)]
        pub user_code_hash: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub completed_user_id: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub mod cli_browser_login {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_browser_logins")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub request_id: String,
        pub request_secret_hash: String,
        pub callback_url: String,
        pub callback_code_hash: Option<String>,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub completed_user_id: Option<String>,
        pub completed_at_unix: Option<i64>,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub mod cli_exchange_grant {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_exchange_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub grant_hash: String,
        pub user_id: String,
        pub created_at_unix: i64,
        pub expires_at_unix: i64,
        pub consumed_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub mod cli_session {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_cli_sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(unique)]
        pub token_hash: String,
        pub user_id: String,
        pub label: String,
        pub created_at_unix: i64,
        pub last_used_at_unix: Option<i64>,
        pub expires_at_unix: i64,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
