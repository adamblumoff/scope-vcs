use super::{MetadataStore, entities};
use crate::{
    domain::requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, RequestActorRole, RequestAudience, RequestState,
    },
    error::ApiError,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, sea_query::Expr,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyRequestQueueCursor {
    pub snapshot_version: u64,
    pub stake_credits: u32,
    pub ready_at_unix: u64,
    pub request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyRequestQueueRow {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author_user_id: String,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub head_oid: String,
    pub stake_credits: u32,
    pub ready_at_unix: u64,
    pub is_held: bool,
    pub updated_at_unix: u64,
    pub has_git_snapshot: bool,
    snapshot_version: u64,
}

impl ReadyRequestQueueRow {
    pub fn cursor(&self) -> ReadyRequestQueueCursor {
        ReadyRequestQueueCursor {
            snapshot_version: self.snapshot_version,
            stake_credits: self.stake_credits,
            ready_at_unix: self.ready_at_unix,
            request_id: self.id.clone(),
        }
    }
}

#[derive(FromQueryResult)]
struct ReadyRequestQueueDbRow {
    id: String,
    name: String,
    title: String,
    author_user_id: String,
    author_role: String,
    audience: String,
    head_oid: String,
    current_stake_credits: i32,
    ready_at_unix: i64,
    is_held: bool,
    updated_at_unix: i64,
    has_git_snapshot: bool,
}

impl ReadyRequestQueueDbRow {
    fn try_into_read_model(self) -> Result<ReadyRequestQueueRow, ApiError> {
        Ok(ReadyRequestQueueRow {
            id: self.id,
            name: self.name,
            title: self.title,
            author_user_id: self.author_user_id,
            author_role: entities::decode_enum(self.author_role)?,
            audience: entities::decode_enum(self.audience)?,
            head_oid: self.head_oid,
            stake_credits: entities::i32_to_u32(
                self.current_stake_credits,
                "ready request stake credits",
            )?,
            ready_at_unix: entities::i64_to_u64(self.ready_at_unix, "ready request queue time")?,
            is_held: self.is_held,
            updated_at_unix: entities::i64_to_u64(self.updated_at_unix, "request update time")?,
            has_git_snapshot: self.has_git_snapshot,
            snapshot_version: 0,
        })
    }
}

impl MetadataStore {
    pub async fn ready_request_queue_page(
        &self,
        repo_id: &str,
        after: Option<&ReadyRequestQueueCursor>,
        limit: u64,
    ) -> Result<Vec<ReadyRequestQueueRow>, ApiError> {
        let snapshot_version = match after {
            Some(cursor) => cursor.snapshot_version,
            None => ready_queue_snapshot_version(self.db.as_ref(), repo_id).await?,
        };
        let snapshot_version_i64 = i64::try_from(snapshot_version).map_err(|_| {
            ApiError::internal_message(
                "ready request cursor snapshot exceeds PostgreSQL bigint range",
            )
        })?;
        let mut query = entities::request::Entity::find()
            .select_only()
            .column(entities::request::Column::Id)
            .column(entities::request::Column::Name)
            .column(entities::request::Column::Title)
            .column(entities::request::Column::AuthorUserId)
            .column(entities::request::Column::AuthorRole)
            .column(entities::request::Column::Audience)
            .column(entities::request::Column::HeadOid)
            .column(entities::request::Column::CurrentStakeCredits)
            .column(entities::request::Column::ReadyAtUnix)
            .expr_as(
                Expr::col(entities::request::Column::HeldAtUnix).is_not_null(),
                "is_held",
            )
            .column(entities::request::Column::UpdatedAtUnix)
            .expr_as(
                Expr::col(entities::request::Column::GitSnapshot).is_not_null(),
                "has_git_snapshot",
            )
            .filter(entities::request::Column::RepoId.eq(repo_id))
            .filter(
                entities::request::Column::Audience
                    .eq(entities::encode_enum(RequestAudience::Public)?),
            )
            .filter(entities::request::Column::ReadyQueueVersion.lte(snapshot_version_i64))
            .filter(
                entities::request::Column::State
                    .eq(entities::encode_enum(RequestState::ReadyForReview)?),
            );
        if let Some(after) = after {
            let stake_credits = i32::try_from(after.stake_credits).map_err(|_| {
                ApiError::internal_message(
                    "ready request cursor stake exceeds PostgreSQL integer range",
                )
            })?;
            let ready_at_unix = i64::try_from(after.ready_at_unix).map_err(|_| {
                ApiError::internal_message(
                    "ready request cursor time exceeds PostgreSQL bigint range",
                )
            })?;
            query = query.filter(
                Condition::any()
                    .add(entities::request::Column::CurrentStakeCredits.lt(stake_credits))
                    .add(
                        Condition::all()
                            .add(entities::request::Column::CurrentStakeCredits.eq(stake_credits))
                            .add(entities::request::Column::ReadyAtUnix.gt(ready_at_unix)),
                    )
                    .add(
                        Condition::all()
                            .add(entities::request::Column::CurrentStakeCredits.eq(stake_credits))
                            .add(entities::request::Column::ReadyAtUnix.eq(ready_at_unix))
                            .add(entities::request::Column::Id.gt(after.request_id.as_str())),
                    ),
            );
        }
        query
            .order_by_desc(entities::request::Column::CurrentStakeCredits)
            .order_by_asc(entities::request::Column::ReadyAtUnix)
            .order_by_asc(entities::request::Column::Id)
            .limit(limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64))
            .into_model::<ReadyRequestQueueDbRow>()
            .all(self.db.as_ref())
            .await
            .map_err(ApiError::internal)?
            .into_iter()
            .map(|row| {
                row.try_into_read_model().map(|mut row| {
                    row.snapshot_version = snapshot_version;
                    row
                })
            })
            .collect()
    }
}

#[derive(FromQueryResult)]
struct ReadyQueueVersionDbRow {
    snapshot_version: i64,
}

// Ready-cycle versions are retained after exit, so MAX across all request history is
// the monotonic repository watermark. Schema and domain invariants couple publication
// to a non-null version; there is no legacy-data compatibility path in this pre-alpha.
async fn ready_queue_snapshot_version<C>(conn: &C, repo_id: &str) -> Result<u64, ApiError>
where
    C: ConnectionTrait,
{
    entities::request::Entity::find()
        .select_only()
        .expr_as(
            Expr::cust("COALESCE(MAX(ready_queue_version), 0)"),
            "snapshot_version",
        )
        .filter(entities::request::Column::RepoId.eq(repo_id))
        .into_model::<ReadyQueueVersionDbRow>()
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal_message("ready queue snapshot query returned no row"))
        .and_then(|row| entities::i64_to_u64(row.snapshot_version, "ready queue snapshot version"))
}

pub(super) async fn next_ready_queue_version<C>(conn: &C, repo_id: &str) -> Result<u64, ApiError>
where
    C: ConnectionTrait,
{
    ready_queue_snapshot_version(conn, repo_id)
        .await?
        .checked_add(1)
        .ok_or_else(|| ApiError::internal_message("ready queue version overflow"))
        .and_then(|version| {
            i64::try_from(version).map(|_| version).map_err(|_| {
                ApiError::internal_message("ready queue version exceeds PostgreSQL bigint range")
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::TestDatabaseTarget,
        domain::{
            policy::Visibility,
            requests::{RequestActorRole, StartRequestInput},
            store::{AppCatalog, RepoPublicationState, StoredRepository, UserAccount, app_catalog},
        },
    };

    #[tokio::test]
    async fn queue_orders_and_seeks_by_stake_time_and_id() {
        let store = postgres_store();
        for (id, stake_credits, ready_at_unix) in [
            ("req_high", 25, 30),
            ("req_early", 10, 10),
            ("req_tie_a", 10, 20),
            ("req_tie_b", 10, 20),
            ("req_low", 1, 5),
        ] {
            start_ready_request(&store, "owner/repo", id, stake_credits, ready_at_unix).await;
        }
        store
            .mutate_request_for_tests("req_tie_b", |request| {
                request.held_at_unix = Some(21);
                request.held_by_user_id = Some("user_owner".to_string());
                request.updated_at_unix = 21;
            })
            .await
            .unwrap();
        start_private_ready_request(&store, "req_private", 35).await;
        start_ready_request(&store, "owner/other", "req_other", 25, 3).await;
        start_working_request(&store, "req_working").await;

        let first = store
            .ready_request_queue_page("owner/repo", None, 2)
            .await
            .unwrap();
        assert_eq!(queue_ids(&first), ["req_high", "req_early"]);
        assert_eq!(
            first[1].cursor(),
            ReadyRequestQueueCursor {
                snapshot_version: 6,
                stake_credits: 10,
                ready_at_unix: 10,
                request_id: "req_early".to_string(),
            }
        );

        let second = store
            .ready_request_queue_page("owner/repo", Some(&first[1].cursor()), 2)
            .await
            .unwrap();
        assert_eq!(queue_ids(&second), ["req_tie_a", "req_tie_b"]);
        assert!(second[1].is_held);

        let third = store
            .ready_request_queue_page("owner/repo", Some(&second[1].cursor()), 2)
            .await
            .unwrap();
        assert_eq!(queue_ids(&third), ["req_low"]);

        let all_ids = first
            .iter()
            .chain(&second)
            .chain(&third)
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            all_ids,
            ["req_high", "req_early", "req_tie_a", "req_tie_b", "req_low",]
        );
    }

    #[tokio::test]
    async fn cursor_excludes_ready_cycles_created_after_the_first_page() {
        let store = postgres_store();
        start_ready_request(&store, "owner/repo", "req_first", 10, 10).await;
        start_ready_request(&store, "owner/repo", "req_second", 10, 20).await;

        let first = store
            .ready_request_queue_page("owner/repo", None, 1)
            .await
            .unwrap();
        assert_eq!(queue_ids(&first), ["req_first"]);
        let cursor = first[0].cursor();

        start_ready_request(&store, "owner/repo", "req_new_priority", 25, 30).await;
        start_ready_request(&store, "owner/repo", "req_new_tail", 5, 5).await;

        let remaining = store
            .ready_request_queue_page("owner/repo", Some(&cursor), 10)
            .await
            .unwrap();
        assert_eq!(queue_ids(&remaining), ["req_second"]);
    }

    #[tokio::test]
    async fn cursor_is_stable_when_requests_leave_and_reenter_ready() {
        let store = postgres_store();
        start_ready_request(&store, "owner/repo", "req_seen", 20, 10).await;
        start_ready_request(&store, "owner/repo", "req_moves_higher", 10, 20).await;
        start_ready_request(&store, "owner/repo", "req_stable", 5, 30).await;

        let first = store
            .ready_request_queue_page("owner/repo", None, 1)
            .await
            .unwrap();
        assert_eq!(queue_ids(&first), ["req_seen"]);
        let cursor = first[0].cursor();
        assert_eq!(cursor.snapshot_version, 3);

        reenter_ready_request(&store, "req_seen", 1, 40).await;
        reenter_ready_request(&store, "req_moves_higher", 25, 50).await;

        let remaining = store
            .ready_request_queue_page("owner/repo", Some(&cursor), 10)
            .await
            .unwrap();
        assert_eq!(queue_ids(&remaining), ["req_stable"]);

        let fresh = store
            .ready_request_queue_page("owner/repo", None, 10)
            .await
            .unwrap();
        assert_eq!(
            queue_ids(&fresh),
            ["req_moves_higher", "req_stable", "req_seen"]
        );
        assert_eq!(fresh[0].cursor().snapshot_version, 5);
    }

    fn postgres_store() -> MetadataStore {
        let store = MetadataStore::connect_fresh_for_tests(
            &TestDatabaseTarget::required().expect("test database target"),
        )
        .expect("connect test database");
        store
            .seed_catalog_for_tests(catalog_with_repos())
            .expect("seed test catalog");
        store
    }

    fn catalog_with_repos() -> AppCatalog {
        let mut catalog = app_catalog();
        let owner = UserAccount {
            id: "user_owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@scope.test".to_string(),
            email_verified: true,
        };
        catalog.users.insert(owner.id.clone(), owner.clone());
        catalog.users.insert(
            "user_public".to_string(),
            UserAccount {
                id: "user_public".to_string(),
                handle: "public".to_string(),
                email: "public@scope.test".to_string(),
                email_verified: true,
            },
        );
        for name in ["repo", "other"] {
            let mut repo = StoredRepository::new(&owner, name, Visibility::Public).unwrap();
            repo.record.publication_state = RepoPublicationState::Published;
            catalog.repositories.insert(repo.record.id.clone(), repo);
        }
        catalog
    }

    async fn start_working_request(store: &MetadataStore, request_id: &str) {
        store
            .start_request(start_input("owner/repo", request_id))
            .await
            .unwrap();
    }

    async fn start_ready_request(
        store: &MetadataStore,
        repo_id: &str,
        request_id: &str,
        stake_credits: u32,
        ready_at_unix: u64,
    ) {
        store
            .start_request(start_input(repo_id, request_id))
            .await
            .unwrap();
        let ready_queue_version = next_ready_queue_version(store.db.as_ref(), repo_id)
            .await
            .unwrap();
        store
            .mutate_request_for_tests(request_id, |request| {
                request.state = RequestState::ReadyForReview;
                request.ready_queue_version = Some(ready_queue_version);
                request.current_stake_credits = stake_credits;
                request.first_ready_at_unix = Some(ready_at_unix);
                request.ready_at_unix = Some(ready_at_unix);
                request.updated_at_unix = ready_at_unix;
            })
            .await
            .unwrap();
    }

    async fn start_private_ready_request(
        store: &MetadataStore,
        request_id: &str,
        ready_at_unix: u64,
    ) {
        let mut input = start_input("owner/repo", request_id);
        input.author_user_id = "user_owner".to_string();
        input.author_role = RequestActorRole::Owner;
        input.audience = RequestAudience::Private;
        store.start_request(input).await.unwrap();
        let ready_queue_version = next_ready_queue_version(store.db.as_ref(), "owner/repo")
            .await
            .unwrap();
        store
            .mutate_request_for_tests(request_id, |request| {
                request.state = RequestState::ReadyForReview;
                request.ready_queue_version = Some(ready_queue_version);
                request.first_ready_at_unix = Some(ready_at_unix);
                request.ready_at_unix = Some(ready_at_unix);
                request.updated_at_unix = ready_at_unix;
            })
            .await
            .unwrap();
    }

    async fn reenter_ready_request(
        store: &MetadataStore,
        request_id: &str,
        stake_credits: u32,
        ready_at_unix: u64,
    ) {
        store
            .mutate_request_for_tests(request_id, |request| {
                request.state = RequestState::Working;
                request.current_stake_credits = 0;
                request.ready_at_unix = None;
                request.held_at_unix = None;
                request.held_by_user_id = None;
                request.updated_at_unix = ready_at_unix - 1;
            })
            .await
            .unwrap();

        let request = store.request_by_id(request_id).await.unwrap().unwrap();
        let ready_queue_version = next_ready_queue_version(store.db.as_ref(), &request.repo_id)
            .await
            .unwrap();
        store
            .mutate_request_for_tests(request_id, |request| {
                request.state = RequestState::ReadyForReview;
                request.ready_queue_version = Some(ready_queue_version);
                request.current_stake_credits = stake_credits;
                request.ready_at_unix = Some(ready_at_unix);
                request.held_at_unix = None;
                request.held_by_user_id = None;
                request.updated_at_unix = ready_at_unix;
            })
            .await
            .unwrap();
    }

    fn start_input(repo_id: &str, request_id: &str) -> StartRequestInput {
        StartRequestInput {
            id: request_id.to_string(),
            repo_id: repo_id.to_string(),
            name: format!("request-{}", request_id.replace('_', "-")),
            author_user_id: "user_public".to_string(),
            title: Some(format!("Request {request_id}")),
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid: "base".to_string(),
            event_id: format!("event-{request_id}"),
            now_unix: 2,
        }
    }

    fn queue_ids<const N: usize>(requests: &[ReadyRequestQueueRow]) -> [&str; N] {
        requests
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }
}
