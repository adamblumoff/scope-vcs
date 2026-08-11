use super::{
    GeneratedIdKind, GeneratedIdSource, RunStore,
    cleanup_queue::queue_pending_source_blob_deletion_rows,
    entities,
    generated_ids::generate_id,
    object_references::{delete_object_reference, insert_object_reference},
    outbox::ClaimedOutboxJob,
    runs::enqueue_run_in_transaction,
};
use crate::error::{PostgresError, PostgresErrorKind};
use scope_domain::{
    projection::ProjectionViewKey,
    runs::{
        run::{Run, RunSource, RunTrigger},
        trigger::{
            PushTriggerCheck, PushTriggerEvaluation, PushTriggerEvaluationState, PushTriggerInput,
        },
    },
    store::{GitHead, SourceBlob},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub(super) const JOB_KIND: &str = "push_main_trigger_evaluation";

#[derive(Deserialize)]
struct PushMainTriggerJobPayload {
    workflow_schema_version: u8,
    manifest: SourceBlob,
    input: PushTriggerInput,
}

pub async fn enqueue_push_main_trigger_evaluation<C>(
    conn: &C,
    repo_id: &str,
    head: &GitHead,
    input: &PushTriggerInput,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    if head.head_oid != input.head_oid || head.manifest.git_oid != input.head_oid {
        return Err(PostgresError::invalid_input(
            "push trigger input does not match the accepted Git head",
        ));
    }
    let evaluation =
        PushTriggerEvaluation::pending(repo_id, head.change_version, &head.head_oid, now_unix)
            .map_err(PostgresError::from)?;
    entities::push_trigger_evaluation::Model::from_domain(&evaluation)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(PostgresError::internal)?;
    let job = entities::outbox_job::Model::push_main_trigger_evaluation(
        generate_id(generated_ids, GeneratedIdKind::OutboxJob)?,
        JOB_KIND,
        repo_id,
        head.change_version,
        &head.manifest,
        input,
        now_unix,
    )?;
    entities::outbox_job::Entity::insert(job.into_active_model())
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    let reference_id = format!("{repo_id}:{}", head.change_version);
    insert_object_reference(conn, "push_trigger_source", &reference_id, &head.manifest).await?;
    insert_object_reference(conn, "push_trigger_source", &reference_id, &input.snapshot).await
}

pub(super) async fn evaluate<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait + TransactionTrait,
{
    let payload = load_job_payload(conn, &job.id).await?;
    let change_version = u64::try_from(job.repo_version)
        .map_err(|_| PostgresError::internal_message("push trigger change version is negative"))?;
    let tx = conn.begin().await.map_err(PostgresError::internal)?;
    let Some(model) = entities::push_trigger_evaluation::Entity::find_by_id((
        job.repo_id.clone(),
        job.repo_version,
    ))
    .lock_exclusive()
    .one(&tx)
    .await
    .map_err(PostgresError::internal)?
    else {
        release_push_trigger_sources(
            &tx,
            &job.repo_id,
            change_version,
            &payload,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(());
    };
    let mut evaluation = model.try_into_domain()?;
    if evaluation.state != PushTriggerEvaluationState::Pending {
        release_push_trigger_sources(
            &tx,
            &job.repo_id,
            change_version,
            &payload,
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(());
    }
    let revisions = if let Some(message) = &payload.input.configuration_error {
        evaluation
            .configuration_error(message.clone(), now_unix)
            .map_err(PostgresError::from)?;
        Vec::new()
    } else {
        let files = payload
            .input
            .workflows
            .iter()
            .map(|workflow| (workflow.path.as_str(), workflow.bytes.as_slice()));
        match scope_run_config::parse_workflow_set(&job.repo_id, files) {
            Ok(revisions) => revisions,
            Err(error) => {
                evaluation
                    .configuration_error(error.to_string(), now_unix)
                    .map_err(PostgresError::from)?;
                Vec::new()
            }
        }
    };
    let revisions = revisions
        .into_iter()
        .filter(|revision| revision.definition().triggers().push_main())
        .collect::<Vec<_>>();
    let mut checks = Vec::new();
    if evaluation.state == PushTriggerEvaluationState::Pending
        && !revisions.is_empty()
        && let Err(error) = super::runner_protocol_cutover::guard_push_trigger_enqueue(&tx).await
    {
        if error.kind != PostgresErrorKind::Unavailable {
            return Err(error);
        }
        evaluation
            .fail(error.message, now_unix)
            .map_err(PostgresError::from)?;
    }
    if evaluation.state == PushTriggerEvaluationState::Pending {
        for revision in revisions {
            let path = revision.workflow().path().as_str().to_string();
            let idempotency_key = format!(
                "push-main:{}:{}:{}",
                change_version,
                payload.input.head_oid,
                revision.workflow().path().as_str()
            );
            let run_id = stable_push_run_id(&job.repo_id, &idempotency_key);
            let source = RunSource::accepted_revision(
                change_version,
                payload.manifest.clone(),
                payload.input.snapshot.clone(),
                ProjectionViewKey::Private,
            )
            .map_err(PostgresError::from)?;
            let run = Run::new(
                run_id,
                idempotency_key,
                revision.workflow().clone(),
                revision.digest(),
                RunTrigger::PushMain,
                None,
                source,
                None,
                now_unix,
            )
            .map_err(PostgresError::from)?;
            let stored = enqueue_run_in_transaction(&tx, run, revision).await?;
            checks.push(PushTriggerCheck {
                workflow_path: path,
                workflow_name: stored.workflow.path().name().to_string(),
                run_id: stored.id,
            });
        }
        evaluation
            .succeed(checks, now_unix)
            .map_err(PostgresError::from)?;
    }
    save_evaluation(&tx, &evaluation).await?;
    release_push_trigger_sources(
        &tx,
        &job.repo_id,
        change_version,
        &payload,
        now_unix,
        generated_ids,
    )
    .await?;
    tx.commit().await.map_err(PostgresError::internal)
}

async fn load_job_payload<C>(
    conn: &C,
    job_id: &str,
) -> Result<PushMainTriggerJobPayload, PostgresError>
where
    C: ConnectionTrait,
{
    let payload = entities::outbox_job::Entity::find_by_id(job_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("push trigger outbox job not found"))?;
    let payload: PushMainTriggerJobPayload =
        serde_json::from_value(payload.payload).map_err(PostgresError::internal)?;
    if payload.workflow_schema_version
        != entities::outbox_job::PUSH_MAIN_TRIGGER_WORKFLOW_SCHEMA_VERSION
    {
        return Err(PostgresError::internal_message(
            "unsupported push trigger workflow schema version",
        ));
    }
    Ok(payload)
}

pub(super) async fn mark_terminal_failure(
    tx: &DatabaseTransaction,
    job: &ClaimedOutboxJob,
    error: String,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError> {
    let payload = load_job_payload(tx, &job.id).await?;
    let change_version = u64::try_from(job.repo_version)
        .map_err(|_| PostgresError::internal_message("push trigger change version is negative"))?;
    let Some(model) = entities::push_trigger_evaluation::Entity::find_by_id((
        job.repo_id.clone(),
        job.repo_version,
    ))
    .lock_exclusive()
    .one(tx)
    .await
    .map_err(PostgresError::internal)?
    else {
        release_push_trigger_sources(
            tx,
            &job.repo_id,
            change_version,
            &payload,
            now_unix,
            generated_ids,
        )
        .await?;
        return Ok(());
    };
    let mut evaluation = model.try_into_domain()?;
    if evaluation.state == PushTriggerEvaluationState::Pending {
        evaluation
            .fail(error, now_unix)
            .map_err(PostgresError::from)?;
        save_evaluation(tx, &evaluation).await?;
    }
    release_push_trigger_sources(
        tx,
        &job.repo_id,
        change_version,
        &payload,
        now_unix,
        generated_ids,
    )
    .await?;
    Ok(())
}

async fn release_push_trigger_sources<C>(
    conn: &C,
    repo_id: &str,
    change_version: u64,
    payload: &PushMainTriggerJobPayload,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    delete_object_reference(
        conn,
        "push_trigger_source",
        &format!("{repo_id}:{change_version}"),
    )
    .await?;
    queue_pending_source_blob_deletion_rows(
        conn,
        [payload.manifest.clone(), payload.input.snapshot.clone()],
        now_unix,
        generated_ids,
    )
    .await
}

async fn save_evaluation(
    tx: &DatabaseTransaction,
    evaluation: &PushTriggerEvaluation,
) -> Result<(), PostgresError> {
    entities::push_trigger_evaluation::Entity::update(
        entities::push_trigger_evaluation::Model::from_domain(evaluation)?
            .into_active_model()
            .reset_all(),
    )
    .exec(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn stable_push_run_id(repo_id: &str, idempotency_key: &str) -> String {
    let digest = Sha256::digest(format!("{repo_id}\0{idempotency_key}").as_bytes());
    format!("run_push_{}", hex::encode(digest))
}

impl RunStore {
    pub async fn push_trigger_evaluation(
        &self,
        repository_id: &str,
        head_oid: &str,
    ) -> Result<Option<PushTriggerEvaluation>, PostgresError> {
        entities::push_trigger_evaluation::Entity::find()
            .filter(entities::push_trigger_evaluation::Column::RepoId.eq(repository_id.to_string()))
            .filter(entities::push_trigger_evaluation::Column::HeadOid.eq(head_oid.to_string()))
            .order_by_desc(entities::push_trigger_evaluation::Column::ChangeVersion)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(entities::push_trigger_evaluation::Model::try_into_domain)
            .transpose()
    }

    pub async fn runs_by_ids(&self, run_ids: &[String]) -> Result<Vec<Run>, PostgresError> {
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }
        entities::run::Entity::find()
            .filter(entities::run::Column::Id.is_in(run_ids.to_vec()))
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MetadataStore;
    use scope_domain::{
        content_ref::ContentRef,
        policy::Visibility,
        runs::trigger::PushWorkflowFile,
        store::{DEFAULT_GIT_FILE_MODE, UserAccount},
    };
    use sea_orm::{DatabaseBackend, Statement};

    #[tokio::test]
    async fn evaluation_uses_each_pinned_head_and_enqueues_once() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "1111111111111111111111111111111111111111";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(head_oid, 1),
            &PushTriggerInput::new(
                head_oid,
                trigger_snapshot(head_oid, 'a'),
                vec![
                    PushWorkflowFile::new(
                        "/.scope/runs/test.yml",
                        br#"
name: Pinned Test
on: { push: true }
runs-on: any
container: { image: alpine:3.20 }
resources: { cpu: 1, memory: 1gb }
timeout: 1m
caches: []
jobs:
  backend:
    steps:
      - { name: Backend, run: "true" }
  web:
    needs: [backend]
    steps:
      - { name: Web, run: "true" }
"#
                        .to_vec(),
                    )
                    .unwrap(),
                ],
                None,
            )
            .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
        let payload = entities::outbox_job::Entity::find()
            .filter(entities::outbox_job::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::outbox_job::Column::RepoVersion.eq(1))
            .filter(entities::outbox_job::Column::Kind.eq(JOB_KIND))
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap()
            .payload;
        assert_eq!(
            payload["workflow_schema_version"],
            entities::outbox_job::PUSH_MAIN_TRIGGER_WORKFLOW_SCHEMA_VERSION
        );
        let later_head_oid = "3333333333333333333333333333333333333333";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(later_head_oid, 3),
            &PushTriggerInput::new(
                later_head_oid,
                trigger_snapshot(later_head_oid, 'c'),
                Vec::new(),
                None,
            )
            .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

        let summary = store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Succeeded);
        assert_eq!(evaluation.checks.len(), 1);
        let run = store
            .runs()
            .run(&evaluation.checks[0].run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.trigger, RunTrigger::PushMain);
        assert_eq!(run.source.git_oid(), head_oid);
        let jobs = entities::run_job::Entity::find()
            .filter(entities::run_job::Column::RunId.eq(run.id.clone()))
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert_eq!(jobs.len(), 2);
        let later = store
            .runs()
            .push_trigger_evaluation(&repo_id, later_head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(later.state, PushTriggerEvaluationState::Succeeded);
        assert!(later.checks.is_empty());
        assert_eq!(
            object_reference_count(&store, "push_trigger_source", &format!("{repo_id}:1")).await,
            0
        );
        assert_eq!(
            object_reference_count(&store, "push_trigger_source", &format!("{repo_id}:3")).await,
            0
        );
        assert_eq!(
            object_reference_count(&store, "run_source", &run.id).await,
            2
        );

        store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        assert_eq!(store.runs().runs_by_ids(&[run.id]).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invalid_workflow_is_a_configuration_error() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "2222222222222222222222222222222222222222";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(head_oid, 2),
            &PushTriggerInput::new(
                head_oid,
                trigger_snapshot(head_oid, 'b'),
                vec![
                    PushWorkflowFile::new(
                        "/.scope/runs/broken.yml",
                        b"name: broken\nunknown: true\n".to_vec(),
                    )
                    .unwrap(),
                ],
                None,
            )
            .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
        store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();

        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluation.state,
            PushTriggerEvaluationState::ConfigurationError
        );
        assert!(
            evaluation
                .message
                .unwrap()
                .contains("workflow YAML is invalid")
        );
        assert!(evaluation.checks.is_empty());
        assert_eq!(
            object_reference_count(&store, "push_trigger_source", &format!("{repo_id}:2")).await,
            0
        );
        let cleanup = store
            .cleanup()
            .source_blob_cleanup_batch(now() + 301, &crate::db::generated_ids::test_generated_id)
            .await
            .unwrap();
        assert_eq!(cleanup.pending.len(), 2);
    }

    #[tokio::test]
    async fn fenced_cutover_fails_multi_job_push_without_retrying() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        store
            .db
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "UPDATE scope_runner_protocol_cutover SET state = 'v8-fenced' WHERE key = 'current'"
                    .to_string(),
            ))
            .await
            .unwrap();
        let head_oid = "6666666666666666666666666666666666666666";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(head_oid, 6),
            &PushTriggerInput::new(
                head_oid,
                trigger_snapshot(head_oid, 'f'),
                vec![
                    PushWorkflowFile::new(
                        "/.scope/runs/checks.yml",
                        br#"
name: Checks
on: { push: true }
runs-on: any
container: { image: alpine:3.20 }
resources: { cpu: 1, memory: 1gb }
timeout: 1m
caches: []
jobs:
  backend:
    steps:
      - { name: Backend, run: "true" }
  web:
    needs: [backend]
    steps:
      - { name: Web, run: "true" }
"#
                        .to_vec(),
                    )
                    .unwrap(),
                ],
                None,
            )
            .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

        let summary = store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        assert!(summary.completed >= 1);
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Failed);
        assert_eq!(
            evaluation.message.as_deref(),
            Some(
                "push-triggered workflows are blocked while runner protocol cutover is v8-fenced; upgrade a runner and complete the canary suite"
            )
        );
        assert!(evaluation.checks.is_empty());
        let push_job = entities::outbox_job::Entity::find()
            .filter(entities::outbox_job::Column::RepoId.eq(repo_id))
            .filter(entities::outbox_job::Column::RepoVersion.eq(6))
            .filter(entities::outbox_job::Column::Kind.eq(JOB_KIND))
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(push_job.state, "succeeded");
        assert_eq!(push_job.attempts, 0);
    }

    #[tokio::test]
    async fn fenced_cutover_allows_push_without_matching_workflows() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        store
            .db
            .execute(Statement::from_string(
                DatabaseBackend::Postgres,
                "UPDATE scope_runner_protocol_cutover SET state = 'v8-fenced' WHERE key = 'current'"
                    .to_string(),
            ))
            .await
            .unwrap();
        let head_oid = "7777777777777777777777777777777777777777";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(head_oid, 7),
            &PushTriggerInput::new(
                head_oid,
                trigger_snapshot(head_oid, 'a'),
                vec![
                    PushWorkflowFile::new(
                        "/.scope/runs/manual.yml",
                        br#"
name: Manual only
on: { manual: true }
runs-on: any
container: { image: alpine:3.20 }
resources: { cpu: 1, memory: 1gb }
timeout: 1m
caches: []
jobs:
  checks:
    steps:
      - { name: Test, run: "true" }
"#
                        .to_vec(),
                    )
                    .unwrap(),
                ],
                None,
            )
            .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();

        let summary = store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        assert!(summary.completed >= 1);
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Succeeded);
        assert!(evaluation.checks.is_empty());
        assert!(evaluation.message.is_none());
    }

    #[tokio::test]
    async fn terminal_failure_releases_pinned_trigger_sources() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "5555555555555555555555555555555555555555";
        enqueue_push_main_trigger_evaluation(
            store.db.as_ref(),
            &repo_id,
            &trigger_head(head_oid, 5),
            &PushTriggerInput::new(head_oid, trigger_snapshot(head_oid, 'e'), Vec::new(), None)
                .unwrap(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
        let persisted_job = entities::outbox_job::Entity::find()
            .filter(entities::outbox_job::Column::RepoId.eq(repo_id.clone()))
            .filter(entities::outbox_job::Column::Kind.eq(JOB_KIND))
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        let tx = store.db.begin().await.unwrap();
        mark_terminal_failure(
            &tx,
            &ClaimedOutboxJob {
                id: persisted_job.id,
                kind: JOB_KIND.to_string(),
                repo_id: repo_id.clone(),
                repo_version: 5,
                attempts: 12,
            },
            "terminal failure".to_string(),
            now(),
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.state, PushTriggerEvaluationState::Failed);
        assert_eq!(
            object_reference_count(&store, "push_trigger_source", &format!("{repo_id}:5")).await,
            0
        );
        let cleanup = store
            .cleanup()
            .source_blob_cleanup_batch(now() + 301, &crate::db::generated_ids::test_generated_id)
            .await
            .unwrap();
        assert_eq!(cleanup.pending.len(), 2);
    }

    #[tokio::test]
    async fn returning_to_an_earlier_head_creates_a_new_evaluation_and_run() {
        let target = crate::db::TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let repo_id = seed_repo(&store).await;
        let head_oid = "4444444444444444444444444444444444444444";
        let workflow = || {
            PushWorkflowFile::new(
                "/.scope/runs/repeat.yml",
                br#"
name: Repeat
on: { push: true }
runs-on: any
container: { image: alpine:3.20 }
resources: { cpu: 1, memory: 1gb }
timeout: 1m
caches: []
jobs:
  checks:
    steps:
      - { name: Test, run: "true" }
"#
                .to_vec(),
            )
            .unwrap()
        };
        for change_version in [2, 4] {
            enqueue_push_main_trigger_evaluation(
                store.db.as_ref(),
                &repo_id,
                &trigger_head(head_oid, change_version),
                &PushTriggerInput::new(
                    head_oid,
                    trigger_snapshot(
                        head_oid,
                        char::from_digit(change_version as u32, 10).unwrap(),
                    ),
                    vec![workflow()],
                    None,
                )
                .unwrap(),
                now(),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        }

        let summary = store
            .jobs()
            .run_ready_outbox_jobs(
                "push-worker",
                10,
                &|| Ok(now()),
                &crate::db::generated_ids::test_generated_id,
            )
            .await
            .unwrap();
        assert_eq!(summary.failed, 0);
        let evaluation = store
            .runs()
            .push_trigger_evaluation(&repo_id, head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(evaluation.change_version, 4);
        assert_eq!(evaluation.checks.len(), 1);
        let runs = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(repo_id))
            .all(store.db.as_ref())
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[0].id, runs[1].id);
    }

    async fn seed_repo(store: &MetadataStore) -> String {
        let owner = UserAccount {
            id: "user_owner".to_string(),
            handle: "owner".to_string(),
            email: "owner@example.com".to_string(),
            email_verified: true,
        };
        let mut catalog = crate::db::CatalogFixture::default();
        let repo = catalog
            .create_repository(&owner, "repo", Visibility::Private)
            .unwrap()
            .clone();
        catalog.users.insert(owner.id.clone(), owner);
        store.admin().seed_catalog_for_tests(catalog).unwrap();
        repo.record.id
    }

    async fn object_reference_count(store: &MetadataStore, ref_kind: &str, ref_id: &str) -> usize {
        entities::object_reference::Entity::find()
            .filter(entities::object_reference::Column::RefKind.eq(ref_kind.to_string()))
            .filter(entities::object_reference::Column::RefId.eq(ref_id.to_string()))
            .all(store.db.as_ref())
            .await
            .unwrap()
            .len()
    }

    fn trigger_head(head_oid: &str, change_version: u64) -> GitHead {
        let digest = format!("{change_version:x}").repeat(64);
        GitHead {
            head_oid: head_oid.to_string(),
            segment_sequence: change_version,
            change_version,
            manifest: SourceBlob {
                content_ref: ContentRef::git_manifest_sha256(digest.clone()),
                sha256: digest,
                git_oid: head_oid.to_string(),
                git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
                size_bytes: 1,
            },
        }
    }

    fn trigger_snapshot(head_oid: &str, digest_character: char) -> SourceBlob {
        let digest = digest_character.to_string().repeat(64);
        SourceBlob {
            content_ref: ContentRef::git_bundle_sha256(digest.clone()),
            sha256: digest,
            git_oid: head_oid.to_string(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
            size_bytes: 1,
        }
    }

    fn now() -> u64 {
        1_700_000_000
    }
}
