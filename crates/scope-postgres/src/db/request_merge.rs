//! Atomic repository content merge plus request completion.

use super::{
    GeneratedIdSource, RequestStore, acquire_aggregate_lock,
    content_push_transactions::{RepositoryContentSnapshots, accept_and_persist_request_merge},
    entities,
    request_access::ensure_user_exists,
    request_rows::request_by_id,
    request_submission_transactions::persist_lifecycle_mutation,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        landing_file::RepositoryLandingFileMutation,
        repository::RepoLifecycleState,
        repository::git::GitHead,
        repository::updates::RequestMergeOrigin,
        requests::{MergeRequestInput, RequestLifecycleMutation, merge_request},
        reviewed_updates::content::ReviewedUpdateInput,
        runs::catalog::RepositoryWorkflowCatalog,
    },
};

#[derive(Clone, Debug)]
pub struct MergeRequestContentMutation {
    pub request: RequestLifecycleMutation,
    pub git_head: GitHead,
}

impl RequestStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn merge_request_content(
        &self,
        owner: &str,
        name: &str,
        expected_manifest_ref: &scope_domain::content_ref::ContentRef,
        expected_repo_change_version: u64,
        expected_request_head_oid: &str,
        update: ReviewedUpdateInput,
        landing_file_mutation: RepositoryLandingFileMutation,
        workflow_catalog: RepositoryWorkflowCatalog,
        origin: RequestMergeOrigin,
        mut input: MergeRequestInput,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<MergeRequestContentMutation, PostgresError> {
        let now_unix = input.now_unix;
        let repo_id = scope_domain::repository::repo_id(owner, name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.request_id).await?;

        let request = request_by_id(&tx, &input.request_id)
            .await?
            .filter(|request| request.repo_id == repo_id)
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        if request.head_oid != expected_request_head_oid {
            return Err(PostgresError::conflict(
                "request changed since merge was prepared; retry merge",
            ));
        }
        ensure_user_exists(&tx, &input.actor_user_id).await?;

        let repo_row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let repo_change_version = u64::try_from(repo_row.change_version).map_err(|_| {
            PostgresError::internal_message("repository change version is negative")
        })?;
        if repo_change_version != expected_repo_change_version {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let publication_state: RepoLifecycleState = serde_json::from_value(
            serde_json::Value::String(repo_row.publication_state.clone()),
        )
        .map_err(PostgresError::internal)?;
        if publication_state != RepoLifecycleState::Ready {
            return Err(PostgresError::conflict("repo must be ready before merge"));
        }
        let head = entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::conflict("repo has no accepted Git head"))?
            .try_into_domain()?;
        if &head.manifest.content_ref != expected_manifest_ref {
            return Err(PostgresError::conflict(
                "repo changed since merge was prepared; retry merge",
            ));
        }
        let is_member = entities::repository_member::Entity::find()
            .filter(entities::repository_member::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::repository_member::Column::UserId.eq(input.actor_user_id.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some();
        input.actor_is_maintainer = repo_row.owner_user_id == input.actor_user_id || is_member;
        input.merged_head_oid = expected_request_head_oid.to_string();
        input.merged_main_oid = update.git_head.head_oid.clone();
        let request_mutation = merge_request(&request, input)?;

        let git_head = accept_and_persist_request_merge(
            &tx,
            repo_row,
            update,
            RepositoryContentSnapshots {
                landing_file_mutation,
                workflow_catalog,
            },
            origin,
            now_unix,
            generated_ids,
        )
        .await?;

        persist_lifecycle_mutation(&tx, &request_mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MergeRequestContentMutation {
            request: request_mutation,
            git_head,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::db::requests::tests::{postgres_store, start_public_request};
    use scope_domain::{
        content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
        content_ref::ContentRef,
        landing_file::RepositoryLandingFileMutation,
        policy::ScopePath,
        repository::{
            git::{GitHead, GitPackSpan},
            updates::RequestMergeOrigin,
        },
        requests::{MergeRequestInput, RequestState},
        reviewed_updates::content::{
            ReviewedContentChange, ReviewedUpdateInput, apply_reviewed_update_to_repo,
        },
        runs::catalog::RepositoryWorkflowCatalog,
    };

    const BASE_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MERGED_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";

    #[tokio::test]
    async fn locked_merge_derives_maintainer_authorization_before_content_persistence() {
        let store = merge_store().await;
        let prepared = merge_preparation(&store).await;

        let error = store
            .requests()
            .merge_request_content(
                "owner",
                "repo",
                &prepared.expected_manifest_ref,
                prepared.expected_repo_change_version,
                "head",
                prepared.update,
                RepositoryLandingFileMutation::Unchanged,
                prepared.workflow_catalog,
                RequestMergeOrigin::Private {
                    request_id: "req_1".to_string(),
                    request_head_oid: "head".to_string(),
                },
                merge_input("user_public"),
                &super::super::generated_ids::test_generated_id,
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.kind,
            crate::error::PostgresErrorKind::PermissionDenied
        );
        assert_eq!(error.message, "repo maintainer required");
        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.git_head.unwrap().head_oid, BASE_HEAD);
        assert_eq!(
            store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap()
                .state(),
            RequestState::Open
        );
    }

    #[tokio::test]
    async fn locked_merge_allows_owner_and_persists_content_and_request_once() {
        let store = merge_store().await;
        let prepared = merge_preparation(&store).await;

        let mutation = store
            .requests()
            .merge_request_content(
                "owner",
                "repo",
                &prepared.expected_manifest_ref,
                prepared.expected_repo_change_version,
                "head",
                prepared.update,
                RepositoryLandingFileMutation::Unchanged,
                prepared.workflow_catalog,
                RequestMergeOrigin::Private {
                    request_id: "req_1".to_string(),
                    request_head_oid: "head".to_string(),
                },
                merge_input("user_owner"),
                &super::super::generated_ids::test_generated_id,
            )
            .await
            .unwrap();

        assert_eq!(mutation.request.request.state(), RequestState::Merged);
        assert_eq!(mutation.git_head.head_oid, MERGED_HEAD);
        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.git_head.unwrap().head_oid, MERGED_HEAD);
        assert_eq!(repo.graph.commits.len(), 2);
    }

    struct MergePreparation {
        expected_manifest_ref: ContentRef,
        expected_repo_change_version: u64,
        update: ReviewedUpdateInput,
        workflow_catalog: RepositoryWorkflowCatalog,
    }

    async fn merge_store() -> super::super::MetadataStore {
        let store = postgres_store();
        let mut repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        let initial_update = reviewed_update(
            &repo,
            BASE_HEAD,
            1,
            None,
            "/.scope/RULES.md",
            source_blob("rules-content"),
        );
        stage_segment(&store, &initial_update.git_pack_span.segment).await;
        apply_reviewed_update_to_repo(&mut repo, initial_update).unwrap();
        store
            .repositories()
            .replace_repository_for_tests(repo)
            .await
            .unwrap();
        start_public_request(&store).await;
        store
            .requests()
            .mutate_request_for_tests("req_1", |request| {
                request.submitted_at_unix = Some(4);
                request.updated_at_unix = 4;
            })
            .await
            .unwrap();
        store
    }

    async fn merge_preparation(store: &super::super::MetadataStore) -> MergePreparation {
        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        let update = reviewed_update(
            &repo,
            MERGED_HEAD,
            2,
            Some(BASE_HEAD),
            "/README.md",
            source_blob("merged-content"),
        );
        stage_segment(store, &update.git_pack_span.segment).await;
        let workflow_catalog = RepositoryWorkflowCatalog::captured(
            "owner/repo",
            MERGED_HEAD,
            repo.record.change_version + 1,
            Vec::new(),
        )
        .unwrap();
        MergePreparation {
            expected_manifest_ref: repo.git_head.as_ref().unwrap().manifest.content_ref.clone(),
            expected_repo_change_version: repo.record.change_version,
            update,
            workflow_catalog,
        }
    }

    async fn stage_segment(
        store: &super::super::MetadataStore,
        segment: &scope_domain::repository::git::GitSegmentRef,
    ) {
        let repositories = store.repositories();
        repositories
            .begin_git_segment_upload(
                "owner/repo",
                &segment.segment_id,
                &format!("git/segments/v2/owner/repo/{}", segment.segment_id),
                segment.encoding_version,
                1,
            )
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_ready(segment, 2, 2)
            .await
            .unwrap();
    }

    fn reviewed_update(
        repo: &scope_domain::repository::Repository,
        head_oid: &str,
        sequence: u64,
        base_oid: Option<&str>,
        path: &str,
        content: SourceBlob,
    ) -> ReviewedUpdateInput {
        let mut manifest = source_blob(&format!("manifest-{head_oid}"));
        manifest.git_oid = head_oid.to_string();
        let segment = scope_domain::repository::git::GitSegmentRef {
            segment_id: format!("segment-{head_oid}"),
            sha256: "c".repeat(64),
            plaintext_bytes: 1,
            encoding_version: 2,
        };
        ReviewedUpdateInput {
            branch: "refs/heads/main".to_string(),
            author_id: "user_owner".to_string(),
            message: format!("update {head_oid}"),
            git_head: GitHead {
                head_oid: head_oid.to_string(),
                push_sequence: sequence,
                change_version: repo.record.change_version + 1,
                manifest,
            },
            git_pack_span: GitPackSpan {
                first_sequence: sequence,
                last_sequence: sequence,
                geometric_tier: 0,
                base_oid: base_oid.map(str::to_string),
                head_oid: head_oid.to_string(),
                segment,
            },
            changes: vec![ReviewedContentChange {
                path: ScopePath::parse(path).unwrap(),
                content: Some(content),
            }],
            previous_config: Some(repo.repo_config.clone()),
            config: repo.repo_config.clone(),
        }
    }

    fn source_blob(label: &str) -> SourceBlob {
        SourceBlob {
            content_ref: ContentRef::git_bundle_sha256(format!("sha256-{label}")),
            sha256: format!("sha256-{label}"),
            git_oid: label.to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        }
    }

    fn merge_input(actor_user_id: &str) -> MergeRequestInput {
        MergeRequestInput {
            request_id: "req_1".to_string(),
            actor_user_id: actor_user_id.to_string(),
            actor_is_maintainer: false,
            merged_head_oid: String::new(),
            merged_main_oid: String::new(),
            merged_event_id: format!("event_merged_{actor_user_id}"),
            now_unix: 5,
        }
    }
}
