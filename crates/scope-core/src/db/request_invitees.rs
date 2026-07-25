use super::{
    MetadataStore, entities,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
};
use crate::{
    domain::{
        requests::{
            AddRequestInviteeInput, LeaveRequestInput, RemoveRequestInviteeInput, RequestInvitee,
            add_request_invitee as add_invitee, leave_request,
            remove_request_invitee as remove_invitee,
        },
        store::UserAccount,
    },
    error::ApiError,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, TransactionTrait,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestInviteeRead {
    pub invitee: RequestInvitee,
    pub user: UserAccount,
}

#[derive(Clone, Debug)]
pub struct AddRequestInviteeCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub target_handle: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RemoveRequestInviteeCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub target_handle: String,
}

#[derive(Clone, Debug)]
pub struct LeaveRequestCommand {
    pub request_id: String,
    pub actor_user_id: String,
}

impl MetadataStore {
    pub async fn request_invitees(
        &self,
        request_id: &str,
    ) -> Result<Vec<RequestInviteeRead>, ApiError> {
        request_invitee_reads(self.db.as_ref(), request_id).await
    }

    pub async fn request_is_invitee(
        &self,
        request_id: &str,
        user_id: &str,
    ) -> Result<bool, ApiError> {
        request_is_invitee(self.db.as_ref(), request_id, user_id).await
    }

    pub async fn add_request_invitee(
        &self,
        command: AddRequestInviteeCommand,
    ) -> Result<RequestInviteeRead, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &command.request_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let decision =
            request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        ensure_exact_visibility(decision.exact_visible)?;
        let target = user_by_exact_handle(&tx, &command.target_handle)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?;
        let mut invitees = request_invitee_map(&tx, &request.id).await?;
        let invitee = add_invitee(
            &request,
            &mut invitees,
            AddRequestInviteeInput {
                actor_user_id: command.actor_user_id,
                target_user_id: target.id.clone(),
                actor_can_manage_invitees: decision.permissions.can_manage_invitees,
                target_is_maintainer: repo.is_maintainer_user_id(&target.id),
                now_unix: command.now_unix,
            },
        )?;
        insert_request_invitee(&tx, &invitee).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(RequestInviteeRead {
            invitee,
            user: target,
        })
    }

    pub async fn remove_request_invitee(
        &self,
        command: RemoveRequestInviteeCommand,
    ) -> Result<RequestInviteeRead, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &command.request_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let decision =
            request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        ensure_exact_visibility(decision.exact_visible)?;
        let target = user_by_exact_handle(&tx, &command.target_handle)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?;
        let mut invitees = request_invitee_map(&tx, &request.id).await?;
        let invitee = remove_invitee(
            &request,
            &mut invitees,
            RemoveRequestInviteeInput {
                actor_user_id: command.actor_user_id,
                target_user_id: target.id.clone(),
                actor_can_manage_invitees: decision.permissions.can_manage_invitees,
            },
        )?;
        delete_request_invitee(&tx, &invitee.request_id, &invitee.user_id).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(RequestInviteeRead {
            invitee,
            user: target,
        })
    }

    pub async fn leave_request(
        &self,
        command: LeaveRequestCommand,
    ) -> Result<RequestInviteeRead, ApiError> {
        let tx = self.db.begin().await.map_err(ApiError::internal)?;
        let (repo, request) = lock_request_repository(&tx, &command.request_id).await?;
        let actor = user_by_id(&tx, &command.actor_user_id)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?;
        let decision =
            request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        ensure_exact_visibility(decision.exact_visible)?;
        let mut invitees = request_invitee_map(&tx, &request.id).await?;
        let invitee = leave_request(
            &request,
            &mut invitees,
            LeaveRequestInput {
                actor_user_id: command.actor_user_id,
                actor_can_leave_request: decision.permissions.can_leave_request,
            },
        )?;
        delete_request_invitee(&tx, &invitee.request_id, &invitee.user_id).await?;
        tx.commit().await.map_err(ApiError::internal)?;
        Ok(RequestInviteeRead {
            invitee,
            user: actor,
        })
    }
}

fn ensure_exact_visibility(visible: bool) -> Result<(), ApiError> {
    if visible {
        Ok(())
    } else {
        Err(ApiError::not_found("request not found"))
    }
}

pub(super) async fn request_invitee_map<C>(
    conn: &C,
    request_id: &str,
) -> Result<BTreeMap<String, RequestInvitee>, ApiError>
where
    C: ConnectionTrait,
{
    Ok(entities::request_invitee::Entity::find()
        .filter(entities::request_invitee::Column::RequestId.eq(request_id.to_string()))
        .order_by_asc(entities::request_invitee::Column::UserId)
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(entities::request_invitee::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|invitee| (invitee.user_id.clone(), invitee))
        .collect())
}

pub(super) async fn request_is_invitee<C>(
    conn: &C,
    request_id: &str,
    user_id: &str,
) -> Result<bool, ApiError>
where
    C: ConnectionTrait,
{
    entities::request_invitee::Entity::find_by_id((request_id.to_string(), user_id.to_string()))
        .one(conn)
        .await
        .map(|row| row.is_some())
        .map_err(ApiError::internal)
}

pub(super) async fn delete_request_invitees<C>(conn: &C, request_id: &str) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_invitee::Entity::delete_many()
        .filter(entities::request_invitee::Column::RequestId.eq(request_id.to_string()))
        .exec(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

pub(super) async fn user_by_exact_handle<C>(
    conn: &C,
    handle: &str,
) -> Result<Option<UserAccount>, ApiError>
where
    C: ConnectionTrait,
{
    entities::user::Entity::find()
        .filter(entities::user::Column::Handle.eq(handle.to_string()))
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::user::Model::try_into_domain)
        .transpose()
}

async fn user_by_id<C>(conn: &C, user_id: &str) -> Result<Option<UserAccount>, ApiError>
where
    C: ConnectionTrait,
{
    entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(ApiError::internal)?
        .map(entities::user::Model::try_into_domain)
        .transpose()
}

async fn request_invitee_reads<C>(
    conn: &C,
    request_id: &str,
) -> Result<Vec<RequestInviteeRead>, ApiError>
where
    C: ConnectionTrait,
{
    let invitees = request_invitee_map(conn, request_id).await?;
    if invitees.is_empty() {
        return Ok(Vec::new());
    }
    let users = entities::user::Entity::find()
        .filter(entities::user::Column::Id.is_in(invitees.keys().cloned()))
        .all(conn)
        .await
        .map_err(ApiError::internal)?
        .into_iter()
        .map(|row| row.try_into_domain())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|user| (user.id.clone(), user))
        .collect::<BTreeMap<_, _>>();
    let mut reads = invitees
        .into_values()
        .map(|invitee| {
            let user = users
                .get(&invitee.user_id)
                .cloned()
                .ok_or_else(|| ApiError::internal_message("request invitee user row is missing"))?;
            Ok(RequestInviteeRead { invitee, user })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    reads.sort_by(|left, right| {
        left.user
            .handle
            .cmp(&right.user.handle)
            .then_with(|| left.user.id.cmp(&right.user.id))
    });
    Ok(reads)
}

async fn insert_request_invitee<C>(conn: &C, invitee: &RequestInvitee) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    entities::request_invitee::Model::from_domain(invitee)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(ApiError::internal)?;
    Ok(())
}

async fn delete_request_invitee<C>(
    conn: &C,
    request_id: &str,
    user_id: &str,
) -> Result<(), ApiError>
where
    C: ConnectionTrait,
{
    let result = entities::request_invitee::Entity::delete_by_id((
        request_id.to_string(),
        user_id.to_string(),
    ))
    .exec(conn)
    .await
    .map_err(ApiError::internal)?;
    if result.rows_affected == 1 {
        Ok(())
    } else {
        Err(ApiError::internal_message(
            "request invitee row disappeared during mutation",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::{
        db::TestDatabaseTarget,
        domain::{
            policy::Visibility,
            requests::{RequestActorRole, RequestAudience, StartRequestInput},
            store::{AppCatalog, RepoPublicationState, StoredRepository, UserAccount, app_catalog},
        },
    };
    use std::sync::Arc;
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn exact_handle_management_returns_canonical_users_and_leave_revokes_membership() {
        let store = postgres_store(3);
        start_public_request(&store, "request_exact").await;

        assert!(
            user_by_exact_handle(store.db.as_ref(), "target-0")
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            user_by_exact_handle(store.db.as_ref(), "Target-0")
                .await
                .unwrap()
                .is_none()
        );
        let error = store
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "request_exact".to_string(),
                actor_user_id: "user_author".to_string(),
                target_handle: "Target-0".to_string(),
                now_unix: 3,
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::NotFound);

        let added = store
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "request_exact".to_string(),
                actor_user_id: "user_author".to_string(),
                target_handle: "target-1".to_string(),
                now_unix: 4,
            })
            .await
            .unwrap();
        assert_eq!(added.user.id, "user_target_1");
        assert_eq!(added.user.handle, "target-1");
        assert_eq!(added.invitee.invited_by_user_id, "user_author");
        assert!(
            store
                .request_is_invitee("request_exact", "user_target_1")
                .await
                .unwrap()
        );
        assert_eq!(
            store.request_invitees("request_exact").await.unwrap(),
            vec![added]
        );

        let hidden = store
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "request_exact".to_string(),
                actor_user_id: "user_target_0".to_string(),
                target_handle: "target-2".to_string(),
                now_unix: 5,
            })
            .await
            .unwrap_err();
        assert_eq!(hidden.kind, ErrorKind::NotFound);

        let left = store
            .leave_request(LeaveRequestCommand {
                request_id: "request_exact".to_string(),
                actor_user_id: "user_target_1".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(left.user.handle, "target-1");
        assert!(
            store
                .request_invitees("request_exact")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn active_invitee_cap_is_serialized_under_repository_and_request_locks() {
        let store = postgres_store(31);
        start_public_request(&store, "request_cap").await;
        for index in 0..29 {
            store
                .add_request_invitee(AddRequestInviteeCommand {
                    request_id: "request_cap".to_string(),
                    actor_user_id: "user_author".to_string(),
                    target_handle: format!("target-{index}"),
                    now_unix: 10 + index as u64,
                })
                .await
                .unwrap();
        }

        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for index in [29, 30] {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                store
                    .add_request_invitee(AddRequestInviteeCommand {
                        request_id: "request_cap".to_string(),
                        actor_user_id: "user_author".to_string(),
                        target_handle: format!("target-{index}"),
                        now_unix: 100 + index as u64,
                    })
                    .await
            }));
        }
        let mut successes = 0;
        let mut cap_failures = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => successes += 1,
                Err(error) if error.message.contains("30 active invitees") => cap_failures += 1,
                Err(error) => panic!("unexpected invite result: {}", error.message),
            }
        }
        assert_eq!((successes, cap_failures), (1, 1));
        assert_eq!(
            store.request_invitees("request_cap").await.unwrap().len(),
            30
        );
    }

    #[tokio::test]
    async fn maintainer_can_manage_invitees_after_work_was_published() {
        let store = postgres_store(2);
        start_public_request(&store, "request_override").await;
        store
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "request_override".to_string(),
                actor_user_id: "user_author".to_string(),
                target_handle: "target-0".to_string(),
                now_unix: 3,
            })
            .await
            .unwrap();
        let mut request = store
            .request_for_tests("request_override")
            .await
            .unwrap()
            .unwrap();
        request.first_ready_at_unix = Some(4);
        request.ready_queue_version = Some(1);
        request.updated_at_unix = 4;
        super::super::request_rows::save_request_row(store.db.as_ref(), &request)
            .await
            .unwrap();
        let removed = store
            .remove_request_invitee(RemoveRequestInviteeCommand {
                request_id: "request_override".to_string(),
                actor_user_id: "user_owner".to_string(),
                target_handle: "target-0".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(removed.user.id, "user_target_0");

        let error = store
            .add_request_invitee(AddRequestInviteeCommand {
                request_id: "request_override".to_string(),
                actor_user_id: "user_author".to_string(),
                target_handle: "owner".to_string(),
                now_unix: 4,
            })
            .await
            .unwrap_err();
        assert!(error.message.contains("maintainers do not need"));
    }

    fn postgres_store(target_count: usize) -> MetadataStore {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        store.seed_catalog_for_tests(catalog(target_count)).unwrap();
        store
    }

    fn catalog(target_count: usize) -> AppCatalog {
        let owner = user("user_owner", "owner");
        let author = user("user_author", "author");
        let mut repo = StoredRepository::new(&owner, "repo", Visibility::Public).unwrap();
        repo.record.publication_state = RepoPublicationState::Published;
        let mut catalog = app_catalog();
        catalog.users.insert(owner.id.clone(), owner);
        catalog.users.insert(author.id.clone(), author);
        for index in 0..target_count {
            let target = user(&format!("user_target_{index}"), &format!("target-{index}"));
            catalog.users.insert(target.id.clone(), target);
        }
        catalog.repositories.insert(repo.record.id.clone(), repo);
        catalog
    }

    async fn start_public_request(store: &MetadataStore, request_id: &str) {
        store
            .start_request(StartRequestInput {
                id: request_id.to_string(),
                repo_id: "owner/repo".to_string(),
                name: request_id.replace('_', "-"),
                author_user_id: "user_author".to_string(),
                title: None,
                author_role: RequestActorRole::Public,
                audience: RequestAudience::Public,
                base_main_oid: "base".to_string(),
                event_id: format!("event_{request_id}"),
                now_unix: 2,
            })
            .await
            .unwrap();
    }

    fn user(id: &str, handle: &str) -> UserAccount {
        UserAccount {
            id: id.to_string(),
            handle: handle.to_string(),
            email: format!("{handle}@example.com"),
            email_verified: true,
        }
    }
}
