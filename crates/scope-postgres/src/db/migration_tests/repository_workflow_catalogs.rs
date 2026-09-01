use super::*;
use crate::db::{
    entities, repository_workflow_catalogs_for_maintenance,
    workflow_catalogs::{apply_repository_workflow_catalog, repository_workflow_catalog},
};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    runs::catalog::{RepositoryWorkflowCatalog, RepositoryWorkflowFile},
};
use sea_orm::{ConnectionTrait, EntityTrait, IntoActiveModel};

const REPO_ID: &str = "workflow-owner/repo";
const HEAD_OID: &str = "1111111111111111111111111111111111111111";
const BLOB_OID: &str = "2222222222222222222222222222222222222222";

async fn insert_legacy_repository(db: &DatabaseConnection) {
    db.execute_unprepared(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('workflow-owner', 'workflow-owner', 'workflow@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy
        ) VALUES (
            'workflow-owner/repo', 'workflow-owner', 'repo', 'workflow-owner', 'Ready',
            7, '{}'::jsonb, '{}'::jsonb
        );
        ",
    )
    .await
    .unwrap();
}

async fn insert_repository(db: &DatabaseConnection) {
    db.execute_unprepared(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('workflow-owner', 'workflow-owner', 'workflow@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'workflow-owner/repo', 'workflow-owner', 'repo', 'workflow-owner', 'Ready',
            7, '{}'::jsonb, '{}'::jsonb, 'repoi_workflow_owner_repo'
        );
        ",
    )
    .await
    .unwrap();
}

async fn insert_captured_catalog(db: &DatabaseConnection) {
    db.execute_unprepared(&format!(
        "
        INSERT INTO scope_repository_workflow_catalogs (
            repo_id, source_head_oid, source_change_version, configuration_error
        ) VALUES ('{REPO_ID}', '{HEAD_OID}', 7, NULL)
        "
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn maintenance_reads_catalogs_from_the_canonical_pre_migration_schema() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(28))
        .await
        .unwrap();
    insert_legacy_repository(db.as_ref()).await;
    let file = RepositoryWorkflowFile::from_content(
        "/.scope/runs/checks.yml",
        DEFAULT_GIT_FILE_MODE,
        b"name: checks\n".to_vec(),
    )
    .unwrap();
    let catalog = RepositoryWorkflowCatalog::captured(REPO_ID, HEAD_OID, 7, vec![file]).unwrap();
    apply_repository_workflow_catalog(db.as_ref(), &catalog)
        .await
        .unwrap();

    let catalogs = repository_workflow_catalogs_for_maintenance(target.schema_database_url())
        .await
        .unwrap();

    assert_eq!(catalogs.len(), 1);
    assert_eq!(catalogs[0].repository_id(), REPO_ID);
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, "m0029_exact_compatible_caches");
}

#[tokio::test]
async fn maintenance_has_no_catalogs_to_read_before_the_catalog_migration() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(27))
        .await
        .unwrap();

    let catalogs = repository_workflow_catalogs_for_maintenance(target.schema_database_url())
        .await
        .unwrap();

    assert!(catalogs.is_empty());
    assert!(!relation_exists(db.as_ref(), "scope_repository_workflow_catalogs").await);
}

#[tokio::test]
async fn repository_workflow_catalog_schema_enforces_identity_bounds_and_cascade() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;
    insert_captured_catalog(db.as_ref()).await;

    db.execute_unprepared(&format!(
        "
        INSERT INTO scope_repository_workflow_files (
            repo_id, path, oid, size_bytes, git_file_mode, content_bytes
        ) VALUES (
            '{REPO_ID}', '/.scope/runs/checks.yml', '{BLOB_OID}',
            4, '100644', decode('74657374', 'hex')
        )
        "
    ))
    .await
    .unwrap();

    for statement in [
        format!(
            "UPDATE scope_repository_workflow_catalogs
             SET source_head_oid = 'not-a-git-oid'
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_catalogs
             SET source_change_version = -1
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_catalogs
             SET configuration_error = ''
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_catalogs
             SET configuration_error = 'rejected'
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_files
             SET path = '/.scope/runs/Nested/checks.yml'
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_files
             SET oid = 'not-a-git-oid'
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_files
             SET size_bytes = 5
             WHERE repo_id = '{REPO_ID}'"
        ),
        format!(
            "UPDATE scope_repository_workflow_files
             SET git_file_mode = '120000'
             WHERE repo_id = '{REPO_ID}'"
        ),
    ] {
        assert!(
            db.execute_unprepared(&statement).await.is_err(),
            "{statement}"
        );
    }

    db.execute_unprepared(&format!(
        "DELETE FROM scope_repositories WHERE id = '{REPO_ID}'"
    ))
    .await
    .unwrap();
    for table in [
        "scope_repository_workflow_catalogs",
        "scope_repository_workflow_files",
    ] {
        let count = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[tokio::test]
async fn repository_workflow_catalog_schema_caps_files_and_rejects_files_for_errors() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;
    insert_captured_catalog(db.as_ref()).await;

    db.execute_unprepared(&format!(
        "
        INSERT INTO scope_repository_workflow_files (
            repo_id, path, oid, size_bytes, git_file_mode, content_bytes
        )
        SELECT
            '{REPO_ID}',
            '/.scope/runs/workflow-' || value || '.yml',
            '{BLOB_OID}',
            0,
            '100644',
            ''::bytea
        FROM generate_series(1, 64) AS value
        "
    ))
    .await
    .unwrap();
    assert!(
        db.execute_unprepared(&format!(
            "
            INSERT INTO scope_repository_workflow_files (
                repo_id, path, oid, size_bytes, git_file_mode, content_bytes
            ) VALUES (
                '{REPO_ID}', '/.scope/runs/overflow.yml', '{BLOB_OID}',
                0, '100644', ''::bytea
            )
            "
        ))
        .await
        .is_err()
    );

    db.execute_unprepared(&format!(
        "
        DELETE FROM scope_repository_workflow_files WHERE repo_id = '{REPO_ID}';
        UPDATE scope_repository_workflow_catalogs
        SET configuration_error = 'too many workflow files'
        WHERE repo_id = '{REPO_ID}';
        "
    ))
    .await
    .unwrap();
    assert!(
        db.execute_unprepared(&format!(
            "
            INSERT INTO scope_repository_workflow_files (
                repo_id, path, oid, size_bytes, git_file_mode, content_bytes
            ) VALUES (
                '{REPO_ID}', '/.scope/runs/checks.yml', '{BLOB_OID}',
                0, '100644', ''::bytea
            )
            "
        ))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn repository_workflow_catalog_replaces_complete_snapshots_and_detects_corruption() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;

    let first_file = RepositoryWorkflowFile::from_content(
        "/.scope/runs/checks.yml",
        DEFAULT_GIT_FILE_MODE,
        b"name: checks\n".to_vec(),
    )
    .unwrap();
    let first =
        RepositoryWorkflowCatalog::captured(REPO_ID, HEAD_OID, 7, vec![first_file]).unwrap();
    apply_repository_workflow_catalog(db.as_ref(), &first)
        .await
        .unwrap();
    assert_eq!(
        repository_workflow_catalog(db.as_ref(), REPO_ID)
            .await
            .unwrap(),
        Some(first)
    );

    let empty = RepositoryWorkflowCatalog::captured(REPO_ID, HEAD_OID, 8, Vec::new()).unwrap();
    apply_repository_workflow_catalog(db.as_ref(), &empty)
        .await
        .unwrap();
    assert_eq!(
        repository_workflow_catalog(db.as_ref(), REPO_ID)
            .await
            .unwrap(),
        Some(empty)
    );
    let file_count = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_repository_workflow_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(file_count, 0);

    let rejected =
        RepositoryWorkflowCatalog::rejected(REPO_ID, HEAD_OID, 9, "too many files").unwrap();
    apply_repository_workflow_catalog(db.as_ref(), &rejected)
        .await
        .unwrap();
    assert_eq!(
        repository_workflow_catalog(db.as_ref(), REPO_ID)
            .await
            .unwrap(),
        Some(rejected)
    );

    apply_repository_workflow_catalog(
        db.as_ref(),
        &RepositoryWorkflowCatalog::captured(
            REPO_ID,
            HEAD_OID,
            10,
            vec![
                RepositoryWorkflowFile::from_content(
                    "/.scope/runs/checks.yml",
                    DEFAULT_GIT_FILE_MODE,
                    b"name: checks\n".to_vec(),
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .await
    .unwrap();
    db.execute_unprepared(&format!(
        "UPDATE scope_repository_workflow_files
         SET content_bytes = repeat('x', size_bytes::integer)::bytea
         WHERE repo_id = '{REPO_ID}'"
    ))
    .await
    .unwrap();
    assert!(
        repository_workflow_catalog(db.as_ref(), REPO_ID)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn repository_workflow_catalog_backfill_rechecks_repository_and_live_blob_identity() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    insert_repository(db.as_ref()).await;

    let bytes = b"name: checks\n".to_vec();
    let file = RepositoryWorkflowFile::from_content(
        "/.scope/runs/checks.yml",
        DEFAULT_GIT_FILE_MODE,
        bytes,
    )
    .unwrap();
    let blob = SourceBlob {
        content_ref: ContentRef::git_blob(file.oid()),
        sha256: String::new(),
        git_oid: file.oid().to_string(),
        git_file_mode: file.git_file_mode().to_string(),
        size_bytes: file.size_bytes(),
    };
    entities::git_head::Entity::insert(
        entities::git_head::Model {
            repo_id: REPO_ID.to_string(),
            head_oid: HEAD_OID.to_string(),
            push_sequence: 1,
            change_version: 7,
            manifest_object_key: serde_json::to_string(&ContentRef::git_manifest_sha256(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ))
            .unwrap(),
            manifest_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            manifest_size_bytes: 1,
        }
        .into_active_model(),
    )
    .exec(db.as_ref())
    .await
    .unwrap();
    entities::live_file::Entity::insert(
        entities::live_file::Model {
            repo_id: REPO_ID.to_string(),
            path: "/.scope/runs/checks.yml".to_string(),
            content: serde_json::to_value(&blob).unwrap(),
        }
        .into_active_model(),
    )
    .exec(db.as_ref())
    .await
    .unwrap();

    let store = crate::db::RepositoryStore {
        db: std::sync::Arc::clone(&db),
        postgres_database_url: None,
    };
    let candidates = store
        .repository_workflow_catalog_backfill_candidates()
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].repo_id, REPO_ID);
    assert_eq!(candidates[0].source_change_version, 7);
    assert_eq!(
        candidates[0].workflow_blobs,
        vec![("/.scope/runs/checks.yml".to_string(), blob)]
    );

    let catalog = RepositoryWorkflowCatalog::captured(REPO_ID, HEAD_OID, 7, vec![file]).unwrap();
    assert!(
        store
            .store_backfilled_repository_workflow_catalog(&catalog)
            .await
            .unwrap()
    );
    assert!(
        !store
            .store_backfilled_repository_workflow_catalog(&catalog)
            .await
            .unwrap()
    );
    store
        .delete_repository_workflow_catalog_for_tests(REPO_ID)
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        "UPDATE scope_repositories SET change_version = 8 WHERE id = '{REPO_ID}'"
    ))
    .await
    .unwrap();
    assert!(
        store
            .store_backfilled_repository_workflow_catalog(&catalog)
            .await
            .unwrap()
    );
    store
        .delete_repository_workflow_catalog_for_tests(REPO_ID)
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        "UPDATE scope_git_heads SET change_version = 8 WHERE repo_id = '{REPO_ID}'"
    ))
    .await
    .unwrap();
    assert!(
        store
            .store_backfilled_repository_workflow_catalog(&catalog)
            .await
            .is_err()
    );
}
